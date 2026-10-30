//! Chat is the primary surface: a calm conversation, inline agent work,
//! permission gates and the composer that drives the selected harness.

use dunlin_agent::{PermissionChoice, PermissionKind, PlanItem, PlanStatus, ToolCallView};
use dunlin_domain::ToolStatus;
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Icon;
use dunlin_ui::component::Sizable;
use dunlin_ui::component::button::{Button, ButtonVariants};
use dunlin_ui::component::input::Textarea;
use dunlin_ui::component::text::TextView;
use dunlin_ui::component::tooltip::Tooltip;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use super::chat_controls;
use crate::app::{DunlinApp, SettingsTab, format_ms, tool_icon};
use crate::harness::{ChatItem, HarnessStatus, QueuedPrompt};

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let connection_problem = connection_problem(this, cx);
    if this.harness.transcript.is_empty() {
        let composer = composer(this, window, cx);
        return div()
            .id("chat-start")
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .bg(p.bg)
            .children(connection_problem)
            .child(
                div()
                    .w_full()
                    .max_w(px(824.))
                    .px(px(22.))
                    .pb(px(18.))
                    .child(
                        div()
                            .text_size(px(25.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.fg)
                            .child("What would you like to work on?"),
                    )
                    .child(
                        div()
                            .mt(px(7.))
                            .text_size(px(13.))
                            .text_color(p.muted_fg)
                            .child("Choose your environment and agent, then send a message."),
                    ),
            )
            .child(composer)
            .into_any_element();
    }
    let messages = conversation(this, cx);
    let composer = composer(this, window, cx);

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(p.bg)
        .children(connection_problem)
        .child(messages)
        .child(composer)
        .into_any_element()
}

fn connection_problem(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> Option<AnyElement> {
    let command = this.harness.missing_command.as_ref()?;
    let p = palette(cx);
    let can_use_opencode =
        this.session_harness() != "OpenCode" && crate::harness::command_available("opencode acp");
    Some(div().w_full().px(px(22.)).py(px(12.)).flex().justify_center().flex_shrink_0().child(
        div().w_full().max_w(px(824.)).p(px(12.)).rounded(px(9.))
            .border_1().border_color(p.amber.opacity(0.35)).bg(p.amber_soft)
            .flex().flex_col().gap(px(8.))
            .child(div().text_size(px(13.)).text_color(p.amber).child(format!("{} needs an ACP adapter", this.session_harness())))
            .child(div().text_size(px(12.)).text_color(p.muted_fg).child(
                "The required ACP command is not installed. No connection is being retried. Your existing conversations are preserved."
            ))
            .child(div().text_size(px(11.5)).font_family(dunlin_ui::theme::code_font()).child(command.clone()))
            .child(div().flex().flex_wrap().gap(px(8.))
                .when(can_use_opencode, |view| view.child(
                    Button::new("missing-adapter-opencode").primary().small().label("Start with OpenCode")
                        .on_click(cx.listener(|this, _, _, cx| this.select_chat_harness("OpenCode", cx)))))
                .child(Button::new("missing-adapter-settings").ghost().small().label("Harness settings")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_open = true;
                        this.settings_tab = SettingsTab::Harnesses;
                        cx.notify();
                    })))
                .child(Button::new("missing-adapter-retry").ghost().small().label("Check again")
                    .on_click(cx.listener(|this, _, _, cx| this.connect_harness(cx)))))
    ).into_any_element())
}

pub(crate) fn reasoning_toggle(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    div()
        .id("reasoning-toggle")
        .h(px(23.))
        .px(px(7.))
        .rounded(px(6.))
        .border_1()
        .border_color(if this.reasoning_open {
            p.violet.opacity(0.35)
        } else {
            p.border
        })
        .bg(if this.reasoning_open {
            p.violet_soft
        } else {
            p.card
        })
        .flex()
        .items_center()
        .gap(px(5.))
        .cursor_pointer()
        .hover(move |style| style.bg(p.hover))
        .on_click(cx.listener(|this, _, _, cx| {
            this.reasoning_open = !this.reasoning_open;
            cx.notify();
        }))
        .child(
            Icon::new(IconName::Brain)
                .size(px(11.5))
                .text_color(if this.reasoning_open {
                    p.violet
                } else {
                    p.muted_fg
                }),
        )
        .child(
            div()
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if this.reasoning_open {
                    p.violet
                } else {
                    p.muted_fg
                })
                .child("Reasoning"),
        )
        .into_any_element()
}

fn conversation(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let session = this.session().id.clone();
    let following = this.chat_scroll.prepare(&session);
    let scroll_probe = this.chat_scroll.handle.clone();
    let items = crate::chat_presentation::project(
        &this.harness.transcript,
        this.final_answer_only,
        this.harness.running,
        this.stream_responses,
    );
    let running = this.harness.running;
    let status = this.harness.status.clone();
    let reasoning_open = this.reasoning_open;
    let expanded = this.expanded_tool;
    let waiting_for_permission = items
        .iter()
        .any(|item| matches!(item, ChatItem::Permission { resolved: None, .. }));

    let mut inner = div()
        .w_full()
        .flex_shrink_0()
        .max_w(px(824.))
        .pb(px(12.))
        .flex()
        .flex_col()
        .gap(px(18.));

    if items.is_empty() {
        inner = inner.child(empty_state(&status, this.project().is_remote(), cx, &p));
    } else {
        let mut index = 0;
        while index < items.len() {
            if let ChatItem::User { text, at_ms, .. } = &items[index] {
                inner = inner.child(user_message(
                    index,
                    text,
                    this.chat_timestamps
                        .then(|| crate::chat_timing::local_time(*at_ms))
                        .flatten(),
                    cx,
                    &p,
                ));
                index += 1;
                continue;
            }

            let start = index;
            while index < items.len() && !matches!(items[index], ChatItem::User { .. }) {
                index += 1;
            }
            let timing = start.checked_sub(1).and_then(|index| {
                if let ChatItem::User { timing, .. } = &items[index] {
                    *timing
                } else {
                    None
                }
            });
            let caption = crate::chat_timing::agent_caption(
                &items[start..index],
                timing,
                this.chat_timestamps,
                this.chat_duration,
            );
            inner = inner.child(agent_group(
                &items[start..index],
                start,
                AgentGroupOptions {
                    reasoning_open,
                    expanded,
                    session: &session,
                    caption,
                    logo: this.chat_logo,
                },
                cx,
                &p,
            ));
        }
    }

    if running && !waiting_for_permission {
        inner = inner.child(running_card(
            this.final_answer_only,
            this.stream_responses,
            &p,
        ));
    }

    let stream = div()
        .id("chat-stream")
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .overflow_y_scroll()
        .track_scroll(&this.chat_scroll.handle)
        .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
            let delta = event.delta.pixel_delta(px(16.)).y;
            if delta > px(0.) {
                this.chat_scroll.pause_following();
                cx.notify();
                return;
            }
            if delta == px(0.) {
                return;
            }
            // Read the resulting offset after GPUI's default wheel handler.
            // Sampling during normal renders would confuse growing content
            // with the reader deliberately scrolling away from the live edge.
            let app = cx.entity().downgrade();
            window.defer(cx, move |_, cx| {
                let _ = app.update(cx, |this, cx| {
                    this.chat_scroll.user_scrolled();
                    cx.notify();
                });
            });
        }))
        .px(px(22.))
        .py(px(18.))
        .flex()
        .flex_col()
        .items_center()
        .child(inner);

    div()
        .relative()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .child(stream)
        .when(std::env::var_os("DUNLIN_TRACE_SCROLL").is_some(), |view| view.child(
            canvas(|_, _, _| (), move |_, _, _, _| {
                crate::logging::info(&format!("chat scroll layout · follow {following} · offset {:.1} · maximum {:.1}", scroll_probe.offset().y / px(1.), scroll_probe.max_offset().y / px(1.)));
            }).absolute().size(px(0.))
        ))
        .when(!following, |view| {
            view.child(
                div()
                    .absolute()
                    .bottom(px(8.))
                    .left_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(
                        Button::new("chat-jump-latest")
                            .small()
                            .icon(IconName::ArrowDown)
                            .label("Jump to latest")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.chat_scroll.jump_to_latest();
                                cx.notify();
                            })),
                    ),
            )
        })
        .into_any_element()
}

fn empty_state(
    status: &HarnessStatus,
    remote: bool,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let (status_color, status_soft) = harness_tone(p, status);
    let starters = [
        (
            "Explain this codebase",
            "Map the architecture, entry points and the riskiest modules.",
            IconName::Map,
            p.teal,
        ),
        (
            "Fix a failing test",
            "Run the suite, find the first failure and propose a minimal fix.",
            IconName::FlaskConical,
            p.violet,
        ),
        (
            "Review my changes",
            "Review the working tree and call out risky edits before I commit.",
            IconName::GitBranch,
            p.blue,
        ),
    ];

    let mut card = div()
        .w_full()
        .pt(px(36.))
        .pb(px(12.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(16.))
        .child(
            div()
                .size(px(42.))
                .rounded(px(12.))
                .bg(p.fg)
                .text_color(p.bg)
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(IconName::Bird).size(px(22.)).text_color(p.bg)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(5.))
                .child(
                    div()
                        .text_size(px(19.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.fg)
                        .child("What should we build?"),
                )
                .child(
                    div()
                        .max_w(px(430.))
                        .text_center()
                        .text_size(px(12.5))
                        .line_height(relative(1.55))
                        .text_color(p.muted_fg)
                        .child(if remote {
                            "Dunlin drives the relay harness over ACP. Files, prompts and permission decisions stay on your server."
                        } else {
                            "Dunlin drives your harness over ACP. Prompts, tool calls and permission decisions stay on this machine."
                        }),
                ),
        );

    let mut starters_col = div().w_full().max_w(px(560.)).flex().flex_col().gap(px(6.));
    for (title, subtitle, icon_name, color) in starters {
        starters_col = starters_col.child(starter_row(title, subtitle, icon_name, color, cx, p));
    }
    card = card.child(starters_col);

    card.child(
        div()
            .mt(px(2.))
            .h(px(28.))
            .px(px(9.))
            .rounded_full()
            .bg(status_soft)
            .border_1()
            .border_color(status_color.opacity(0.25))
            .flex()
            .items_center()
            .gap(px(6.))
            .child(dot(status_color, 5.0))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(status_color)
                    .child(status.label()),
            )
            .when_some(status.detail(), |this, detail| {
                this.child(
                    div()
                        .text_size(px(11.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(status_color.opacity(0.75))
                        .child(detail),
                )
            }),
    )
    .into_any_element()
}

fn starter_row(
    title: &str,
    subtitle: &str,
    icon_name: IconName,
    color: Hsla,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let prompt = subtitle.to_string();
    div()
        .id(format!("starter:{title}"))
        .w_full()
        .px(px(11.))
        .py(px(9.))
        .rounded(px(9.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .flex()
        .items_center()
        .gap(px(10.))
        .cursor_pointer()
        .hover(move |style| style.bg(p.hover))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.composer.update(cx, |state, cx| {
                state.set_value(prompt.clone(), window, cx);
                state.focus(window, cx);
            });
            cx.notify();
        }))
        .child(
            div()
                .size(px(28.))
                .flex_shrink_0()
                .rounded(px(7.))
                .bg(color.opacity(0.12))
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(icon_name).size(px(14.)).text_color(color)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .flex_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.muted_fg)
                        .truncate()
                        .child(subtitle.to_string()),
                ),
        )
        .child(
            Icon::new(IconName::ArrowUpRight)
                .size(px(12.))
                .text_color(p.muted_fg.opacity(0.7)),
        )
        .into_any_element()
}

struct AgentGroupOptions<'a> {
    reasoning_open: bool,
    expanded: Option<usize>,
    session: &'a str,
    caption: Option<String>,
    logo: bool,
}

fn agent_group(
    items: &[ChatItem],
    base: usize,
    options: AgentGroupOptions<'_>,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let AgentGroupOptions {
        reasoning_open,
        expanded,
        session,
        caption,
        logo,
    } = options;
    let mut body = div()
        .pl(px(if logo { 33. } else { 0. }))
        .flex()
        .flex_col()
        .gap(px(14.));

    for (offset, item) in items.iter().enumerate() {
        let index = base + offset;
        match item {
            ChatItem::AgentText {
                text, streaming, ..
            } => {
                if text.is_empty() {
                    continue;
                }
                let copy_text = text.clone();
                let group = format!("agent-text-{index}");
                let mut text_block = div()
                    .id(("agent-text", index))
                    .group(group.clone())
                    .relative()
                    .pr(px(24.))
                    .text_size(px(13.0))
                    .line_height(relative(1.62))
                    .text_color(p.fg.opacity(0.92))
                    .child(markdown_text(
                        format!("response:{session}:{index}"),
                        text.clone(),
                    ));
                if !*streaming {
                    text_block = text_block.child(
                        div()
                            .id(("agent-copy", index))
                            .absolute()
                            .top(px(0.))
                            .right(px(0.))
                            .size(px(21.))
                            .rounded(px(4.))
                            .bg(p.bg)
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .opacity(0.0)
                            .group_hover(group, move |style| style.opacity(1.0))
                            .hover(move |style| style.bg(p.hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                                this.set_notice("Copied the response", cx);
                            }))
                            .child(
                                Icon::new(IconName::Copy)
                                    .size(px(11.))
                                    .text_color(p.muted_fg),
                            ),
                    );
                }
                body = body.child(text_block);
            }
            ChatItem::Thought { text, .. } => {
                body = body.child(thought_card(text, reasoning_open, cx, p));
            }
            ChatItem::Tool {
                view, duration_ms, ..
            } => {
                if crate::projection::child_id(&view.detail).is_some_and(|id| {
                    items
                        .iter()
                        .any(|item| matches!(item,ChatItem::Child {id:child,..} if child==id))
                }) {
                    continue;
                }
                body = body.child(tool_card(index, view, *duration_ms, expanded, cx, p));
            }
            ChatItem::Plan { items, .. } => {
                body = body.child(plan_card(items, p));
            }
            ChatItem::Permission {
                request_id,
                title,
                options,
                resolved,
                ..
            } => {
                body = body.child(permission_card(
                    request_id,
                    title,
                    options,
                    resolved.as_deref(),
                    cx,
                    p,
                ));
            }
            ChatItem::Error { message, .. } => {
                if offset > 0
                    && matches!(&items[offset - 1], ChatItem::Error { message: previous, .. } if previous == message)
                {
                    continue;
                }
                body = body.child(error_card(
                    index,
                    message,
                    repeated_errors(items, offset),
                    p,
                ));
            }
            ChatItem::Child {
                id, title, items, ..
            } => {
                let label = title.clone().unwrap_or_else(|| "Subagent".into());
                let detail = items
                    .iter()
                    .filter_map(|item| match item {
                        ChatItem::AgentText { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<String>();
                let open = expanded == Some(index);
                body = body.child(
                    div()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(p.border)
                        .p(px(8.))
                        .child(
                            Button::new(("child-details", index))
                                .ghost()
                                .small()
                                .label(format!("Subagent · {label}"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.expanded_tool = if open { None } else { Some(index) };
                                    cx.notify();
                                })),
                        )
                        .when(open, |view| {
                            view.child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(p.muted_fg)
                                    .child(id.clone()),
                            )
                            .child(markdown_text(format!("child:{id}"), detail))
                        }),
                );
            }
            ChatItem::User { .. } => {}
        }
    }

    div()
        .flex()
        .flex_col()
        .gap(px(9.))
        .child(agent_header(logo, caption, p))
        .child(body)
        .into_any_element()
}

fn markdown_text(id: String, text: String) -> TextView {
    TextView::markdown(id, text)
        .scrollable(false)
        .selectable(true)
        .on_link_click(|url, _, _, cx| {
            if url::Url::parse(url)
                .is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "mailto"))
            {
                cx.open_url(url);
            }
        })
}

fn user_message(
    index: usize,
    text: &str,
    time: Option<String>,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let copy_text = text.to_string();
    let retry_text = text.to_string();
    let edit_text = text.to_string();
    let group = format!("user-message-{index}");
    let action_group = group.clone();

    div()
        .id(("user-message", index))
        .group(group)
        .w_full()
        .flex()
        .justify_end()
        .child(
            div()
                .max_w(px(620.))
                .px(px(13.))
                .py(px(10.))
                .rounded(px(11.))
                .bg(p.muted)
                .border_1()
                .border_color(p.border)
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(
                    div()
                        .text_size(px(13.0))
                        .line_height(relative(1.6))
                        .text_color(p.fg)
                        .child(text.to_string()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg.opacity(0.75))
                                .child(
                                    time.map(|time| format!("You · {time}"))
                                        .unwrap_or_else(|| "You".into()),
                                ),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .id(("user-copy", index))
                                .size(px(20.))
                                .rounded(px(4.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .opacity(0.0)
                                .group_hover(action_group.clone(), move |style| style.opacity(1.0))
                                .hover(move |style| style.bg(p.card))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copy_text.clone(),
                                    ));
                                    this.set_notice("Copied the prompt", cx);
                                }))
                                .child(
                                    Icon::new(IconName::Copy)
                                        .size(px(11.))
                                        .text_color(p.muted_fg),
                                ),
                        )
                        .child(
                            div()
                                .id(("user-edit", index))
                                .size(px(20.))
                                .rounded(px(4.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .opacity(0.0)
                                .group_hover(action_group.clone(), move |style| style.opacity(1.0))
                                .hover(move |style| style.bg(p.card))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.edit_prompt(edit_text.clone(), window, cx);
                                }))
                                .child(
                                    Icon::new(IconName::Pencil)
                                        .size(px(11.))
                                        .text_color(p.muted_fg),
                                ),
                        )
                        .child(
                            div()
                                .id(("user-retry", index))
                                .size(px(20.))
                                .rounded(px(4.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .opacity(0.0)
                                .group_hover(action_group, move |style| style.opacity(1.0))
                                .hover(move |style| style.bg(p.card))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.retry_prompt(retry_text.clone(), window, cx);
                                }))
                                .child(
                                    Icon::new(IconName::RotateCcw)
                                        .size(px(11.))
                                        .text_color(p.muted_fg),
                                ),
                        ),
                ),
        )
        .into_any_element()
}

fn agent_avatar(p: &Palette) -> AnyElement {
    div()
        .size(px(24.))
        .flex_shrink_0()
        .rounded(px(7.))
        .bg(p.fg)
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(IconName::Bird).size(px(13.)).text_color(p.bg))
        .into_any_element()
}

fn agent_header(logo: bool, caption: Option<String>, p: &Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(7.))
        .when(logo, |view| view.child(agent_avatar(p)))
        .child(
            div()
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.fg)
                .child("Dunlin"),
        )
        .when_some(caption, |view, caption| {
            view.child(
                div()
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .child(caption),
            )
        })
        .into_any_element()
}

fn thought_card(text: &str, open: bool, cx: &mut Context<DunlinApp>, p: &Palette) -> AnyElement {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let line_count = lines.len().max(1);

    let mut card = div()
        .rounded(px(9.))
        .border_1()
        .border_color(p.border)
        .bg(p.subtle)
        .overflow_hidden()
        .child(
            div()
                .id("reasoning-card")
                .h(px(30.))
                .px(px(9.))
                .flex()
                .items_center()
                .gap(px(6.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.reasoning_open = !this.reasoning_open;
                    cx.notify();
                }))
                .child(
                    Icon::new(IconName::Brain)
                        .size(px(12.5))
                        .text_color(p.violet),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg.opacity(0.85))
                        .child("Thought"),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(format!(
                            "{line_count} {}",
                            if line_count == 1 { "step" } else { "steps" }
                        )),
                )
                .child(div().flex_1())
                .child(
                    Icon::new(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(px(12.))
                    .text_color(p.muted_fg),
                ),
        );

    if open {
        let mut body = div()
            .px(px(10.))
            .pb(px(9.))
            .pt(px(2.))
            .flex()
            .flex_col()
            .gap(px(5.))
            .child(div().h(px(1.)).w_full().bg(p.border));
        for (index, line) in lines.iter().enumerate() {
            body = body.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(6.))
                    .child(
                        div()
                            .mt(px(4.))
                            .size(px(4.))
                            .rounded_full()
                            .bg(p.violet.opacity(0.55)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.5))
                            .line_height(relative(1.5))
                            .text_color(p.muted_fg)
                            .child(format!("{}. {}", index + 1, line)),
                    ),
            );
        }
        if lines.is_empty() {
            body = body.child(
                div()
                    .text_size(px(11.5))
                    .text_color(p.muted_fg)
                    .child("Thinking…"),
            );
        }
        card = card.child(body);
    }

    card.into_any_element()
}

fn tool_card(
    index: usize,
    tool: &ToolCallView,
    duration_ms: u64,
    expanded: Option<usize>,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let is_open = expanded == Some(index);
    let (status_color, status_soft) = match tool.status {
        ToolStatus::Running => (p.blue, p.blue_soft),
        ToolStatus::Waiting => (p.amber, p.amber_soft),
        ToolStatus::Done => (p.green, p.green_soft),
        ToolStatus::Failed => (p.red, p.red_soft),
    };
    let icon_name = tool_icon(tool.kind);
    let target = if tool.locations.is_empty() {
        crate::projection::tool_summary(&tool.detail)
            .lines()
            .next()
            .unwrap_or("")
            .trim_start_matches(['#', ' '])
            .to_string()
    } else {
        tool.locations.join(", ")
    };
    let title = tool.title.clone();
    let status = tool.status;
    let detail = tool.detail.clone();
    let additions = tool.additions;
    let deletions = tool.deletions;
    let diff = tool.diff.clone();

    let mut card = div()
        .rounded(px(9.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .overflow_hidden()
        .child(
            div()
                .id(("tool-card", index))
                .min_h(px(34.))
                .px(px(8.))
                .py(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.expanded_tool = if this.expanded_tool == Some(index) {
                        None
                    } else {
                        Some(index)
                    };
                    cx.notify();
                }))
                .child(
                    div()
                        .size(px(22.))
                        .flex_shrink_0()
                        .rounded(px(6.))
                        .bg(p.muted)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(icon_name).size(px(12.)).text_color(p.muted_fg)),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child(title),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(11.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .truncate()
                        .child(target),
                )
                .when(additions > 0 || deletions > 0, |this| {
                    this.child(
                        div()
                            .text_size(px(11.0))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(p.green)
                            .child(format!("+{additions}")),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(p.red)
                            .child(format!("−{deletions}")),
                    )
                })
                .child(
                    div()
                        .h(px(16.))
                        .px(px(5.))
                        .rounded_full()
                        .bg(status_soft)
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .when(status == ToolStatus::Running, |this| {
                            this.child(
                                Icon::new(IconName::LoaderCircle)
                                    .size(px(8.5))
                                    .text_color(status_color),
                            )
                        })
                        .child(
                            div()
                                .text_size(px(10.5))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(status_color)
                                .child(status.label().to_uppercase()),
                        ),
                )
                .when(duration_ms > 0, |this| {
                    this.child(
                        div()
                            .text_size(px(11.0))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(p.muted_fg)
                            .child(format_ms(duration_ms)),
                    )
                })
                .child(
                    Icon::new(if is_open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(px(11.5))
                    .text_color(p.muted_fg),
                ),
        );

    if is_open {
        let mut body = div()
            .px(px(9.))
            .py(px(8.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(12.0))
                    .line_height(relative(1.55))
                    .text_color(p.muted_fg)
                    .child(if detail.is_empty() {
                        "No output yet.".to_string()
                    } else {
                        detail
                    }),
            );

        if !tool.locations.is_empty() {
            body = body.child(
                div()
                    .px(px(7.))
                    .py(px(5.))
                    .rounded(px(5.))
                    .bg(p.subtle)
                    .border_1()
                    .border_color(p.border)
                    .text_size(px(11.5))
                    .font_family(dunlin_ui::theme::code_font())
                    .text_color(p.fg.opacity(0.75))
                    .child(format!("$ {}", tool.locations.join("  "))),
            );
        }

        if let Some(diff) = diff {
            body = body.child(diff_block(index, &diff, cx, p));
        }

        card = card
            .child(div().h(px(1.)).w_full().bg(p.border))
            .child(body);
    }

    card.into_any_element()
}

fn diff_block(index: usize, patch: &str, cx: &mut Context<DunlinApp>, p: &Palette) -> AnyElement {
    let copied = patch.to_string();
    let mut lines = div().flex().flex_col();
    for line in patch.lines().take(600) {
        let color = if line.starts_with("+++") || line.starts_with("---") {
            p.muted_fg
        } else if line.starts_with('+') {
            p.green
        } else if line.starts_with('-') {
            p.red
        } else if line.starts_with("@@") {
            p.blue
        } else {
            p.muted_fg
        };
        lines = lines.child(
            div()
                .text_size(px(11.0))
                .font_family(dunlin_ui::theme::code_font())
                .line_height(relative(1.4))
                .text_color(color)
                .child(line.to_string()),
        );
    }

    div()
        .rounded(px(7.))
        .border_1()
        .border_color(p.border)
        .bg(p.bg)
        .overflow_hidden()
        .child(
            div()
                .h(px(24.))
                .px(px(7.))
                .flex()
                .items_center()
                .gap(px(5.))
                .border_b_1()
                .border_color(p.border)
                .child(
                    div()
                        .text_size(px(10.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child("DIFF"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id(("tool-diff-copy", index))
                        .h(px(18.))
                        .px(px(5.))
                        .rounded(px(3.))
                        .flex()
                        .items_center()
                        .gap(px(3.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
                            this.set_notice("Copied the diff", cx);
                        }))
                        .child(
                            Icon::new(IconName::Copy)
                                .size(px(9.5))
                                .text_color(p.muted_fg),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(p.muted_fg)
                                .child("Copy"),
                        ),
                ),
        )
        .child(
            div()
                .id(("tool-diff", index))
                .max_h(px(280.))
                .overflow_y_scroll()
                .p(px(7.))
                .child(lines),
        )
        .into_any_element()
}

fn plan_card(items: &[PlanItem], p: &Palette) -> AnyElement {
    let mut card = div()
        .rounded(px(9.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .p(px(10.))
        .flex()
        .flex_col()
        .gap(px(7.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    Icon::new(IconName::ListTodo)
                        .size(px(12.))
                        .text_color(p.teal),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child("EXECUTION PLAN"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg.opacity(0.7))
                        .child(format!("{} steps", items.len())),
                ),
        );

    for item in items {
        let (icon_name, color) = match item.status {
            PlanStatus::Completed => (IconName::CircleCheck, p.green),
            PlanStatus::InProgress => (IconName::LoaderCircle, p.blue),
            PlanStatus::Pending => (IconName::Circle, p.muted_fg.opacity(0.55)),
        };
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .child(Icon::new(icon_name).size(px(12.5)).text_color(color))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(12.0))
                        .text_color(if item.status == PlanStatus::Completed {
                            p.muted_fg
                        } else if item.status == PlanStatus::InProgress {
                            p.fg
                        } else {
                            p.muted_fg.opacity(0.75)
                        })
                        .child(item.content.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(if item.status == PlanStatus::InProgress {
                            p.blue
                        } else {
                            p.muted_fg.opacity(0.65)
                        })
                        .child(crate::harness::plan_status_label(item.status)),
                ),
        );
    }

    card.into_any_element()
}

fn permission_card(
    request_id: &str,
    title: &str,
    options: &[PermissionChoice],
    resolved: Option<&str>,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let denied = resolved
        .map(|label| label.starts_with("Denied"))
        .unwrap_or(false);
    let accent = if denied { p.red } else { p.amber };

    let mut card = div()
        .rounded(px(9.))
        .border_1()
        .border_color(accent.opacity(0.35))
        .bg(if denied { p.red_soft } else { p.amber_soft })
        .overflow_hidden()
        .child(
            div()
                .px(px(10.))
                .py(px(8.))
                .flex()
                .flex_col()
                .gap(px(7.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            Icon::new(if denied {
                                IconName::ShieldX
                            } else {
                                IconName::ShieldCheck
                            })
                            .size(px(13.))
                            .text_color(accent),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(accent)
                                .child(if denied {
                                    "Permission denied"
                                } else if resolved.is_some() {
                                    "Permission granted"
                                } else {
                                    "Permission requested"
                                }),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(accent.opacity(0.7))
                                .child(if resolved.is_some() {
                                    "resolved"
                                } else {
                                    "waiting"
                                }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(11.5))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.fg.opacity(0.8))
                                .truncate()
                                .child(title.to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg.opacity(0.8))
                                .child("ACP"),
                        ),
                ),
        );

    if let Some(label) = resolved {
        card = card.child(
            div()
                .px(px(10.))
                .pb(px(8.))
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    Icon::new(if denied { IconName::X } else { IconName::Check })
                        .size(px(11.))
                        .text_color(accent),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(accent)
                        .child(label.to_string()),
                ),
        );
    } else {
        let mut buttons = div()
            .px(px(10.))
            .pb(px(8.))
            .flex()
            .items_center()
            .gap(px(6.));
        for option in options {
            let request = request_id.to_string();
            let option_id = option.id.clone();
            let allowed = matches!(
                option.kind,
                PermissionKind::AllowOnce | PermissionKind::AllowAlways
            );
            let label = format!(
                "{} · {}",
                if allowed { "Allowed" } else { "Denied" },
                option.label
            );
            let primary = option.kind == PermissionKind::AllowOnce;
            let outline = option.kind == PermissionKind::AllowAlways;

            buttons = buttons.child(
                div()
                    .id(format!("permission:{request_id}:{}", option.id))
                    .h(px(25.))
                    .px(px(10.))
                    .rounded(px(5.))
                    .when(primary, |this| this.bg(p.fg))
                    .when(outline, |this| {
                        this.border_1()
                            .border_color(accent.opacity(0.35))
                            .bg(p.card)
                    })
                    .when(!primary && !outline, |this| {
                        this.border_1().border_color(p.border).bg(p.card)
                    })
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .hover(move |style| style.opacity(0.88))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.resolve_permission(
                            &request,
                            Some(option_id.clone()),
                            label.clone(),
                            cx,
                        );
                    }))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .font_weight(if primary {
                                FontWeight::MEDIUM
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if primary {
                                p.bg
                            } else if allowed {
                                p.fg
                            } else {
                                p.muted_fg
                            })
                            .child(option.label.clone()),
                    ),
            );
        }

        buttons = buttons.child(div().flex_1()).child(
            div()
                .text_size(px(11.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(accent.opacity(0.75))
                .child("session/request_permission"),
        );

        card = card.child(buttons);
    }

    card.into_any_element()
}

fn repeated_errors(items: &[ChatItem], start: usize) -> usize {
    let Some(ChatItem::Error { message, .. }) = items.get(start) else {
        return 0;
    };
    items[start..]
        .iter()
        .take_while(|item| matches!(item, ChatItem::Error { message: next, .. } if next == message))
        .count()
}

fn error_card(index: usize, message: &str, count: usize, p: &Palette) -> AnyElement {
    let details = message.to_string();
    let display = if message.starts_with("ACP connection failed")
        && message.contains("No such file or directory")
    {
        "The harness could not start because its ACP adapter or a required executable was missing."
    } else {
        message
    };
    div()
        .id(("chat-error", index))
        .tooltip(move |window, cx| {
            Tooltip::new(details.clone())
                .max_w(px(600.))
                .build(window, cx)
        })
        .rounded(px(9.))
        .border_1()
        .border_color(p.red.opacity(0.35))
        .bg(p.red_soft)
        .px(px(10.))
        .py(px(8.))
        .flex()
        .items_start()
        .gap(px(7.))
        .child(
            Icon::new(IconName::CircleAlert)
                .size(px(13.))
                .text_color(p.red),
        )
        .child(
            div()
                .flex_1()
                .text_size(px(12.0))
                .line_height(relative(1.5))
                .text_color(p.red)
                .child(display.to_string())
                .when(count > 1, |view| {
                    view.child(
                        div()
                            .mt(px(5.))
                            .text_size(px(11.))
                            .text_color(p.muted_fg)
                            .child(format!(
                                "{count} previous occurrences · grouped from saved history"
                            )),
                    )
                }),
        )
        .into_any_element()
}

fn running_card(final_only: bool, stream: bool, p: &Palette) -> AnyElement {
    div()
        .rounded(px(9.))
        .border_1()
        .border_color(p.blue.opacity(0.28))
        .bg(p.blue_soft)
        .px(px(10.))
        .py(px(8.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    Icon::new(IconName::LoaderCircle)
                        .size(px(12.5))
                        .text_color(p.blue),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.blue)
                        .child("Agent is working"),
                ),
        )
        .child(
            div()
                .text_size(px(11.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.blue.opacity(0.78))
                .child(if !stream {
                    "The final answer will appear when the turn completes"
                } else if final_only {
                    "Response text appears as it arrives; workflow details stay in Graph / Trace"
                } else {
                    "tool calls and reasoning will appear inline"
                }),
        )
        .into_any_element()
}

fn composer(this: &mut DunlinApp, _window: &mut Window, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let running = this.harness.running;
    let has_text = !this.composer.read(cx).value().trim().is_empty();
    let queued = this.harness.queued_prompts();
    let queue_open = this.queue_open;

    let input = Textarea::new(&this.composer)
        .appearance(false)
        .bordered(false)
        .h(px(92.));
    let environment = chat_controls::environment(this, cx);
    let harness = chat_controls::harness(this, cx);
    let mode = chat_controls::mode(this, cx);
    let model = chat_controls::model(this, cx);

    let card = div()
        .w_full()
        .rounded(px(12.))
        .border_1()
        .border_color(if running {
            p.blue.opacity(0.45)
        } else {
            p.border_strong
        })
        .bg(p.card)
        .overflow_hidden()
        .child(
            div()
                .min_h(px(36.))
                .px(px(9.))
                .py(px(3.))
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(5.))
                .bg(p.subtle)
                .border_b_1()
                .border_color(p.border)
                .child(environment)
                .child(div().w(px(1.)).h(px(14.)).bg(p.border))
                .child(harness)
                .children(chat_controls::provider_controls(this, cx))
                .child(div().flex_1())
                .when(!queued.is_empty(), |this| {
                    this.child(
                        div()
                            .id("composer-queue")
                            .h(px(20.))
                            .px(px(6.))
                            .rounded(px(5.))
                            .bg(if queue_open { p.amber } else { p.amber_soft })
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .cursor_pointer()
                            .hover(move |style| style.opacity(0.9))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.queue_open = !this.queue_open;
                                cx.notify();
                            }))
                            .child(
                                Icon::new(IconName::Clock3)
                                    .size(px(10.))
                                    .text_color(if queue_open { p.bg } else { p.amber }),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_family(dunlin_ui::theme::code_font())
                                    .text_color(if queue_open { p.bg } else { p.amber })
                                    .child(format!("{} queued", queued.len())),
                            ),
                    )
                }),
        )
        .child(div().px(px(10.)).pt(px(4.)).child(input))
        .child(
            div()
                .min_h(px(42.))
                .px(px(8.))
                .pb(px(4.))
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(3.))
                .child(mode)
                .child(attachment_button(
                    IconName::Paperclip,
                    "Attach",
                    cx,
                    &p,
                    |this, window, cx| this.attach_files(window, cx),
                ))
                .child(chat_controls::mcp(this, cx))
                .child(div().flex_1())
                .child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(model)
                        .child(
                            div()
                                .id("composer-send")
                                .size(px(27.))
                                .rounded_full()
                                .bg(if running { p.blue } else { p.fg })
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .when(!running && !has_text, |this| this.opacity(0.55))
                                .hover(move |style| style.opacity(0.9))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if this.harness.running {
                                        this.cancel_turn(cx);
                                    } else {
                                        this.send_prompt(window, cx);
                                    }
                                }))
                                .child(
                                    Icon::new(if running {
                                        IconName::CircleStop
                                    } else {
                                        IconName::ArrowUp
                                    })
                                    .size(px(14.))
                                    .text_color(p.bg),
                                ),
                        ),
                ),
        );

    div()
        .flex_shrink_0()
        .w_full()
        .px(px(22.))
        .pt(px(8.))
        .pb(px(9.))
        .flex()
        .justify_center()
        .child(
            div()
                .w_full()
                .max_w(px(824.))
                .flex()
                .flex_col()
                .gap(px(5.))
                .when(queue_open && !queued.is_empty(), |this| {
                    this.child(queue_panel(&queued, cx, &p))
                })
                .child(card)
                .when(this.show_shortcut_hints, |view| {
                    view.child(
                        div()
                            .px(px(2.))
                            .text_right()
                            .text_size(px(11.))
                            .text_color(p.muted_fg.opacity(0.65))
                            .child("Enter to send · Shift+Enter for a new line"),
                    )
                }),
        )
        .into_any_element()
}

fn queue_panel(queued: &[QueuedPrompt], cx: &mut Context<DunlinApp>, p: &Palette) -> AnyElement {
    let mut rows = div()
        .id("prompt-queue-rows")
        .max_h(px(158.))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(2.));

    for (index, prompt) in queued.iter().enumerate() {
        let id = prompt.id;
        let text = prompt.text.clone();
        let first = index == 0;
        let last = index + 1 == queued.len();

        rows = rows.child(
            div()
                .id(("queued-prompt", index))
                .min_h(px(30.))
                .px(px(6.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(5.))
                .hover(move |style| style.bg(p.hover))
                .child(
                    div()
                        .w(px(16.))
                        .flex_shrink_0()
                        .text_size(px(10.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg.opacity(0.6))
                        .child(format!("{}", index + 1)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(px(11.0))
                        .text_color(p.fg.opacity(0.85))
                        .child(text),
                )
                .child(
                    div()
                        .id(("queued-up", index))
                        .size(px(19.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .when(!first, |this| this.hover(move |style| style.bg(p.muted)))
                        .when(first, |this| this.opacity(0.3))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.reorder_queued_prompt(id, -1, cx);
                        }))
                        .child(
                            Icon::new(IconName::ChevronUp)
                                .size(px(10.5))
                                .text_color(p.muted_fg),
                        ),
                )
                .child(
                    div()
                        .id(("queued-down", index))
                        .size(px(19.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .when(!last, |this| this.hover(move |style| style.bg(p.muted)))
                        .when(last, |this| this.opacity(0.3))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.reorder_queued_prompt(id, 1, cx);
                        }))
                        .child(
                            Icon::new(IconName::ChevronDown)
                                .size(px(10.5))
                                .text_color(p.muted_fg),
                        ),
                )
                .child(
                    div()
                        .id(("queued-edit", index))
                        .size(px(19.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.muted))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_queued_prompt(id, window, cx);
                        }))
                        .child(
                            Icon::new(IconName::Pencil)
                                .size(px(10.5))
                                .text_color(p.muted_fg),
                        ),
                )
                .child(
                    div()
                        .id(("queued-remove", index))
                        .size(px(19.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.red_soft))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove_queued_prompt(id, cx);
                        }))
                        .child(Icon::new(IconName::X).size(px(10.5)).text_color(p.red)),
                ),
        );
    }

    div()
        .w_full()
        .rounded(px(10.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .p(px(7.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(10.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child("PROMPT QUEUE"),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg.opacity(0.6))
                        .child(queued.len().to_string()),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("queue-clear")
                        .h(px(20.))
                        .px(px(6.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.red_soft))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.clear_queued_prompts(cx);
                        }))
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(p.muted_fg)
                                .child("Clear"),
                        ),
                )
                .child(
                    div()
                        .id("queue-close")
                        .size(px(20.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.queue_open = false;
                            cx.notify();
                        }))
                        .child(Icon::new(IconName::X).size(px(11.)).text_color(p.muted_fg)),
                ),
        )
        .child(rows)
        .into_any_element()
}

fn attachment_button(
    icon_name: IconName,
    label: &str,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
    action: fn(&mut DunlinApp, &mut Window, &mut Context<DunlinApp>),
) -> AnyElement {
    let label_owned = label.to_string();
    div()
        .id(format!("composer-action:{label}"))
        .h(px(22.))
        .px(px(5.))
        .rounded(px(5.))
        .flex()
        .items_center()
        .gap(px(4.))
        .cursor_pointer()
        .hover(move |style| style.bg(p.hover))
        .on_click(cx.listener(move |this, _, window, cx| {
            action(this, window, cx);
        }))
        .child(Icon::new(icon_name).size(px(11.5)).text_color(p.muted_fg))
        .child(
            div()
                .text_size(px(11.0))
                .text_color(p.muted_fg)
                .child(label_owned),
        )
        .into_any_element()
}

pub(crate) fn harness_tone(p: &Palette, status: &HarnessStatus) -> (Hsla, Hsla) {
    match status {
        HarnessStatus::Ready { .. } => (p.green, p.green_soft),
        HarnessStatus::Starting => (p.amber, p.amber_soft),
        HarnessStatus::Failed(_) | HarnessStatus::Exited(_) => (p.red, p.red_soft),
        HarnessStatus::Offline => (p.muted_fg, p.muted),
    }
}

#[cfg(test)]
mod tests {
    use super::repeated_errors;
    use crate::harness::ChatItem;

    #[test]
    fn repeated_errors_are_grouped_without_crossing_messages_or_other_errors() {
        let error = |message: &str| ChatItem::Error {
            message: message.into(),
            at_ms: 1,
        };
        let items = vec![
            error("missing"),
            error("missing"),
            error("missing"),
            ChatItem::User {
                text: "retry".into(),
                timing: None,
                at_ms: 2,
            },
            error("missing"),
            error("another"),
        ];
        assert_eq!(repeated_errors(&items, 0), 3);
        assert_eq!(repeated_errors(&items, 3), 0);
        assert_eq!(repeated_errors(&items, 4), 1);
        assert_eq!(repeated_errors(&items, 5), 1);
        assert_eq!(repeated_errors(&items, 9), 0);
        assert_eq!(items.len(), 6);
    }
}
