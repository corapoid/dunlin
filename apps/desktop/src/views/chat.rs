//! Chat is the primary surface: a calm conversation, inline agent work,
//! permission gates and the composer that drives the selected harness.

use kestrel_agent::{PermissionChoice, PermissionKind, PlanItem, PlanStatus, ToolCallView};
use kestrel_domain::ToolStatus;
use kestrel_ui::assets::IconName;
use kestrel_ui::component::Icon;
use kestrel_ui::component::input::Textarea;
use kestrel_ui::prelude::FluentBuilder as _;
use kestrel_ui::*;

use crate::app::{KestrelApp, SettingsTab, format_duration, format_ms, format_tokens, tool_icon};
use crate::harness::{ChatItem, HarnessStatus};

pub(crate) fn render(
    this: &mut KestrelApp,
    window: &mut Window,
    cx: &mut Context<KestrelApp>,
) -> AnyElement {
    let p = palette(cx);
    let context = context_bar(this, cx);
    let messages = conversation(this, cx);
    let composer = composer(this, window, cx);

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(p.bg)
        .child(context)
        .child(messages)
        .child(composer)
        .into_any_element()
}

fn context_bar(this: &KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let session = this.session();
    let status = &this.harness.status;
    let (status_color, _) = harness_tone(&p, status);
    let message_count = this.harness.transcript.len();
    let context_label = this
        .harness
        .usage
        .map(|usage| {
            format!(
                "{} / {} ctx",
                format_tokens(usage.used),
                format_tokens(usage.size)
            )
        })
        .unwrap_or_else(|| "context idle".into());

    div()
        .h(px(36.))
        .flex_shrink_0()
        .px(px(18.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(p.border)
        .child(
            Icon::new(IconName::MessageSquareCode)
                .size(px(13.))
                .text_color(p.muted_fg),
        )
        .child(
            div()
                .text_size(px(10.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.fg.opacity(0.88))
                .child("Conversation"),
        )
        .child(
            div()
                .text_size(px(9.5))
                .font_family("Geist Mono")
                .text_color(p.muted_fg)
                .child(format!(
                    "{message_count} {}",
                    if message_count == 1 {
                        "entry"
                    } else {
                        "entries"
                    }
                )),
        )
        .child(div().w(px(1.)).h(px(12.)).bg(p.border))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    Icon::new(IconName::Gauge)
                        .size(px(11.5))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .text_size(px(9.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child(context_label),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(dot(status_color, 4.5))
                .child(
                    div()
                        .text_size(px(9.5))
                        .text_color(p.muted_fg)
                        .child(format!("{} · {}", session.harness, status.label())),
                ),
        )
        .child(div().flex_1())
        .child(suggestion_chip(
            "Explain flow",
            "Explain the ACP permission flow and where child-session requests are dropped.",
            cx,
            &p,
        ))
        .child(suggestion_chip(
            "Review changes",
            "Review the working tree changes and call out risky edits before I commit.",
            cx,
            &p,
        ))
        .child(
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
                .child(Icon::new(IconName::Brain).size(px(11.5)).text_color(
                    if this.reasoning_open {
                        p.violet
                    } else {
                        p.muted_fg
                    },
                ))
                .child(
                    div()
                        .text_size(px(9.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if this.reasoning_open {
                            p.violet
                        } else {
                            p.muted_fg
                        })
                        .child("Reasoning"),
                ),
        )
        .child(
            div()
                .id("compact-chat")
                .h(px(23.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(5.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_notice("Context compaction is handled by the harness", cx);
                }))
                .child(
                    Icon::new(IconName::Minimize2)
                        .size(px(11.5))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .text_size(px(9.5))
                        .text_color(p.muted_fg)
                        .child("Compact"),
                ),
        )
        .into_any_element()
}

fn suggestion_chip(
    label: &str,
    prompt: &str,
    cx: &mut Context<KestrelApp>,
    p: &Palette,
) -> AnyElement {
    let prompt = prompt.to_string();
    div()
        .id(format!("suggestion:{label}"))
        .h(px(23.))
        .px(px(7.))
        .rounded(px(6.))
        .bg(p.card)
        .border_1()
        .border_color(p.border)
        .flex()
        .items_center()
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
                .text_size(px(9.5))
                .text_color(p.muted_fg)
                .child(label.to_string()),
        )
        .into_any_element()
}

fn conversation(this: &mut KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let items = this.harness.transcript.clone();
    let running = this.harness.running;
    let status = this.harness.status.clone();
    let usage = this.harness.usage;
    let session = this.session().clone();
    let reasoning_open = this.reasoning_open;
    let expanded = this.expanded_tool;
    let waiting_for_permission = items
        .iter()
        .any(|item| matches!(item, ChatItem::Permission { resolved: None, .. }));

    let mut inner = div()
        .w_full()
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
            if let ChatItem::User { text, .. } = &items[index] {
                inner = inner.child(user_message(text, &p));
                index += 1;
                continue;
            }

            let start = index;
            while index < items.len() && !matches!(items[index], ChatItem::User { .. }) {
                index += 1;
            }
            inner = inner.child(agent_group(
                &items[start..index],
                start,
                &session,
                &status,
                reasoning_open,
                expanded,
                running,
                cx,
                &p,
            ));
        }
    }

    if running && !waiting_for_permission {
        inner = inner.child(running_card(usage, &p));
    }

    let bottom_hint = div()
        .w_full()
        .max_w(px(824.))
        .pt(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .child(
            Icon::new(if running {
                IconName::LoaderCircle
            } else {
                IconName::ArrowDown
            })
            .size(px(10.))
            .text_color(p.muted_fg),
        )
        .child(
            div()
                .text_size(px(9.))
                .font_family("Geist Mono")
                .text_color(p.muted_fg.opacity(0.7))
                .child(if running {
                    "Streaming from the harness"
                } else {
                    "Conversation is up to date"
                }),
        );

    div()
        .id("chat-stream")
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .overflow_y_scroll()
        .px(px(22.))
        .py(px(18.))
        .flex()
        .flex_col()
        .items_center()
        .child(inner)
        .child(bottom_hint)
        .into_any_element()
}

fn empty_state(
    status: &HarnessStatus,
    remote: bool,
    cx: &mut Context<KestrelApp>,
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
                        .text_size(px(19.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.fg)
                        .child("What should we build?"),
                )
                .child(
                    div()
                        .max_w(px(430.))
                        .text_center()
                        .text_size(px(11.))
                        .line_height(relative(1.55))
                        .text_color(p.muted_fg)
                        .child(if remote {
                            "Kestrel drives the relay harness over ACP. Files, prompts and permission decisions stay on your server."
                        } else {
                            "Kestrel drives your harness over ACP. Prompts, tool calls and permission decisions stay on this machine."
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
                    .text_size(px(9.5))
                    .text_color(status_color)
                    .child(status.label()),
            )
            .when_some(status.detail(), |this, detail| {
                this.child(
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
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
    cx: &mut Context<KestrelApp>,
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
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_size(px(9.5))
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

#[allow(clippy::too_many_arguments)]
fn agent_group(
    items: &[ChatItem],
    base: usize,
    session: &kestrel_domain::SessionSummary,
    status: &HarnessStatus,
    reasoning_open: bool,
    expanded: Option<usize>,
    running: bool,
    cx: &mut Context<KestrelApp>,
    p: &Palette,
) -> AnyElement {
    let mut body = div().pl(px(33.)).flex().flex_col().gap(px(14.));

    for (offset, item) in items.iter().enumerate() {
        let index = base + offset;
        match item {
            ChatItem::AgentText {
                text, streaming, ..
            } => {
                let content = if *streaming {
                    format!("{text} ▍")
                } else {
                    text.clone()
                };
                body = body.child(
                    div()
                        .text_size(px(12.5))
                        .line_height(relative(1.62))
                        .text_color(p.fg.opacity(0.92))
                        .child(content),
                );
            }
            ChatItem::Thought { text, .. } => {
                body = body.child(thought_card(text, reasoning_open, cx, p));
            }
            ChatItem::Tool {
                view, duration_ms, ..
            } => {
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
                body = body.child(error_card(message, p));
            }
            ChatItem::User { .. } => {}
        }
    }

    div()
        .flex()
        .flex_col()
        .gap(px(9.))
        .child(agent_header(session, status, running, p))
        .child(body)
        .into_any_element()
}

fn user_message(text: &str, p: &Palette) -> AnyElement {
    div()
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
                        .text_size(px(12.))
                        .line_height(relative(1.6))
                        .text_color(p.fg)
                        .child(text.to_string()),
                )
                .child(
                    div().flex().justify_end().child(
                        div()
                            .text_size(px(8.5))
                            .font_family("Geist Mono")
                            .text_color(p.muted_fg.opacity(0.75))
                            .child("You"),
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

fn agent_header(
    session: &kestrel_domain::SessionSummary,
    status: &HarnessStatus,
    running: bool,
    p: &Palette,
) -> AnyElement {
    let model = if session.model == "—" || session.model.is_empty() {
        "Auto".to_string()
    } else {
        session.model.clone()
    };

    div()
        .flex()
        .items_center()
        .gap(px(7.))
        .child(agent_avatar(p))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.fg)
                .child("Kestrel"),
        )
        .child(
            div()
                .h(px(17.))
                .px(px(5.))
                .rounded(px(4.))
                .bg(p.violet_soft)
                .flex()
                .items_center()
                .gap(px(4.))
                .child(
                    Icon::new(IconName::Sparkles)
                        .size(px(9.5))
                        .text_color(p.violet),
                )
                .child(
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
                        .text_color(p.violet)
                        .child(model),
                ),
        )
        .child(
            div()
                .text_size(px(8.5))
                .font_family("Geist Mono")
                .text_color(p.muted_fg)
                .child(format!("{} · ACP", session.harness)),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(dot(
                    if running {
                        p.blue
                    } else {
                        p.muted_fg.opacity(0.6)
                    },
                    4.5,
                ))
                .child(
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child(if running {
                            "streaming".to_string()
                        } else if session.duration_secs > 0 {
                            format_duration(session.duration_secs)
                        } else {
                            status.label()
                        }),
                ),
        )
        .into_any_element()
}

fn thought_card(text: &str, open: bool, cx: &mut Context<KestrelApp>, p: &Palette) -> AnyElement {
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
                        .text_size(px(10.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg.opacity(0.85))
                        .child("Thought"),
                )
                .child(
                    div()
                        .text_size(px(9.))
                        .font_family("Geist Mono")
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
                            .text_size(px(10.))
                            .line_height(relative(1.5))
                            .text_color(p.muted_fg)
                            .child(format!("{}. {}", index + 1, line)),
                    ),
            );
        }
        if lines.is_empty() {
            body = body.child(
                div()
                    .text_size(px(10.))
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
    cx: &mut Context<KestrelApp>,
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
        tool.detail.lines().next().unwrap_or("").to_string()
    } else {
        tool.locations.join(", ")
    };
    let title = tool.title.clone();
    let status = tool.status;
    let detail = tool.detail.clone();
    let additions = tool.additions;
    let deletions = tool.deletions;

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
                        .text_size(px(10.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child(title),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(10.))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .truncate()
                        .child(target),
                )
                .when(additions > 0 || deletions > 0, |this| {
                    this.child(
                        div()
                            .text_size(px(8.5))
                            .font_family("Geist Mono")
                            .text_color(p.green)
                            .child(format!("+{additions}")),
                    )
                    .child(
                        div()
                            .text_size(px(8.5))
                            .font_family("Geist Mono")
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
                                .text_size(px(8.))
                                .font_family("Geist Mono")
                                .text_color(status_color)
                                .child(status.label().to_uppercase()),
                        ),
                )
                .when(duration_ms > 0, |this| {
                    this.child(
                        div()
                            .text_size(px(8.5))
                            .font_family("Geist Mono")
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
                    .text_size(px(10.5))
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
                    .text_size(px(9.5))
                    .font_family("Geist Mono")
                    .text_color(p.fg.opacity(0.75))
                    .child(format!("$ {}", tool.locations.join("  "))),
            );
        }

        card = card
            .child(div().h(px(1.)).w_full().bg(p.border))
            .child(body);
    }

    card.into_any_element()
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
                        .text_size(px(9.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child("EXECUTION PLAN"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
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
                        .text_size(px(10.5))
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
                        .text_size(px(9.))
                        .font_family("Geist Mono")
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
    cx: &mut Context<KestrelApp>,
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
                                .text_size(px(10.5))
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
                                .text_size(px(8.5))
                                .font_family("Geist Mono")
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
                                .text_size(px(10.))
                                .font_family("Geist Mono")
                                .text_color(p.fg.opacity(0.8))
                                .truncate()
                                .child(title.to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(9.))
                                .font_family("Geist Mono")
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
                        .text_size(px(9.5))
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
                            .text_size(px(9.5))
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
                .text_size(px(8.5))
                .font_family("Geist Mono")
                .text_color(accent.opacity(0.75))
                .child("session/request_permission"),
        );

        card = card.child(buttons);
    }

    card.into_any_element()
}

fn error_card(message: &str, p: &Palette) -> AnyElement {
    div()
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
                .text_size(px(10.5))
                .line_height(relative(1.5))
                .text_color(p.red)
                .child(message.to_string()),
        )
        .into_any_element()
}

fn running_card(usage: Option<crate::harness::HarnessUsage>, p: &Palette) -> AnyElement {
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
                        .text_size(px(10.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.blue)
                        .child("Agent is working"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
                        .text_color(p.blue.opacity(0.8))
                        .child(
                            usage
                                .map(|usage| {
                                    format!(
                                        "{} / {}",
                                        format_tokens(usage.used),
                                        format_tokens(usage.size)
                                    )
                                })
                                .unwrap_or_else(|| "streaming".into()),
                        ),
                ),
        )
        .child(
            div()
                .text_size(px(9.))
                .font_family("Geist Mono")
                .text_color(p.blue.opacity(0.78))
                .child("tool calls and reasoning will appear inline"),
        )
        .into_any_element()
}

fn composer(
    this: &mut KestrelApp,
    _window: &mut Window,
    cx: &mut Context<KestrelApp>,
) -> AnyElement {
    let p = palette(cx);
    let remote = this.project().is_remote();
    let harness = if remote {
        "Remote".to_string()
    } else {
        this.default_harness.clone()
    };
    let model = this.session().model.clone();
    let permission = this.permission_mode;
    let status = this.harness.status.clone();
    let (status_color, status_soft) = harness_tone(&p, &status);
    let running = this.harness.running;
    let has_text = !this.composer.read(cx).value().trim().is_empty();
    let branch = this
        .project()
        .branch
        .clone()
        .unwrap_or_else(|| "main".into());
    let project = this.project().name.clone();
    let command_count = this.harness.commands.len();

    let input = Textarea::new(&this.composer)
        .appearance(false)
        .bordered(false)
        .h(px(58.));

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
                .h(px(30.))
                .px(px(9.))
                .flex()
                .items_center()
                .gap(px(5.))
                .border_b_1()
                .border_color(p.border)
                .child(
                    div()
                        .id("composer-harness")
                        .h(px(20.))
                        .px(px(6.))
                        .rounded(px(5.))
                        .bg(status_soft)
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.settings_open = true;
                            this.settings_tab = if remote {
                                SettingsTab::Remote
                            } else {
                                SettingsTab::Harnesses
                            };
                            cx.notify();
                        }))
                        .child(dot(status_color, 4.5))
                        .child(
                            div()
                                .text_size(px(9.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(status_color)
                                .child(harness),
                        ),
                )
                .child(
                    div()
                        .id("composer-model")
                        .h(px(20.))
                        .px(px(6.))
                        .rounded(px(5.))
                        .bg(p.muted)
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_notice("Model selection is negotiated by the harness", cx);
                        }))
                        .child(
                            Icon::new(IconName::Sparkles)
                                .size(px(10.))
                                .text_color(p.violet),
                        )
                        .child(
                            div()
                                .text_size(px(9.))
                                .text_color(p.fg.opacity(0.85))
                                .child(model),
                        )
                        .child(
                            Icon::new(IconName::ChevronDown)
                                .size(px(9.5))
                                .text_color(p.muted_fg),
                        ),
                )
                .child(
                    div()
                        .id("composer-permission")
                        .h(px(20.))
                        .px(px(6.))
                        .rounded(px(5.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings_open = true;
                            this.settings_tab = SettingsTab::Permissions;
                            cx.notify();
                        }))
                        .child(
                            Icon::new(IconName::ShieldCheck)
                                .size(px(10.))
                                .text_color(p.muted_fg),
                        )
                        .child(
                            div()
                                .text_size(px(9.))
                                .text_color(p.muted_fg)
                                .child(permission.label()),
                        ),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg.opacity(0.7))
                        .child(if status.is_ready() {
                            format!("ACP · {command_count} commands")
                        } else {
                            "ACP · stdio".to_string()
                        }),
                ),
        )
        .child(div().px(px(10.)).pt(px(4.)).child(input))
        .child(
            div()
                .h(px(36.))
                .px(px(8.))
                .pb(px(4.))
                .flex()
                .items_center()
                .gap(px(3.))
                .child(attachment_button(
                    IconName::Paperclip,
                    "Attach",
                    cx,
                    &p,
                    |this, window, cx| this.attach_files(window, cx),
                ))
                .child(attachment_button(
                    IconName::AtSign,
                    "Mention",
                    cx,
                    &p,
                    |this, window, cx| this.mention_file(window, cx),
                ))
                .child(attachment_button(
                    IconName::SquareTerminal,
                    "Tools",
                    cx,
                    &p,
                    |this, window, cx| this.open_composer_tools(window, cx),
                ))
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg.opacity(0.65))
                        .child("⌘↵"),
                )
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
                .child(card)
                .child(
                    div()
                        .px(px(2.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            Icon::new(IconName::GitBranch)
                                .size(px(10.))
                                .text_color(p.muted_fg),
                        )
                        .child(
                            div()
                                .text_size(px(8.5))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg)
                                .child(format!("{branch} · {project} · workspace")),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(px(8.5))
                                .text_color(if matches!(status, HarnessStatus::Failed(_)) {
                                    p.red
                                } else {
                                    p.muted_fg.opacity(0.65)
                                })
                                .child(status.detail().unwrap_or_else(|| {
                                    "Kestrel can make mistakes. Review tool calls before approving."
                                        .into()
                                })),
                        ),
                ),
        )
        .into_any_element()
}

fn attachment_button(
    icon_name: IconName,
    label: &str,
    cx: &mut Context<KestrelApp>,
    p: &Palette,
    action: fn(&mut KestrelApp, &mut Window, &mut Context<KestrelApp>),
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
                .text_size(px(8.5))
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

pub(crate) fn message_count_label(count: usize) -> String {
    format!(
        "{count} {}",
        if count == 1 { "message" } else { "messages" }
    )
}
