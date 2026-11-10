//! Chat is the primary surface: a calm conversation, inline agent work,
//! permission gates and the composer that drives the selected harness.

use dunlin_agent::{PermissionChoice, PermissionKind, PlanItem, PlanStatus, ToolCallView};
use dunlin_domain::ToolStatus;
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Icon;
use dunlin_ui::component::button::ButtonVariants;
use dunlin_ui::component::text::TextView;
use dunlin_ui::component::tooltip::Tooltip;
use dunlin_ui::component::{Disableable, Selectable, Sizable};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use super::chat_controls;
use crate::app::{DunlinApp, SettingsTab, format_ms, tool_icon};
use crate::harness::{ChatItem, HarnessStatus, QueuedPrompt};

mod text;
mod work;

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let connection_problem = connection_problem(this, cx);
    let coordination = crate::coordination::cards(this, cx);
    if this.harness.transcript.is_empty() {
        let progress = if this.reduce_motion || cx.reduce_motion() {
            1.
        } else {
            this.workbench
                .chat_reveal
                .map(|start| (start.elapsed().as_secs_f32() / 0.28).min(1.))
                .unwrap_or(1.)
        };
        let reveal = 1. - (1. - progress).powi(3);
        if progress < 1. {
            let app = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                let _ = app.update(cx, |_, cx| cx.notify());
            });
            window.request_animation_frame();
        } else {
            this.workbench.chat_reveal = None;
        }
        let composer = composer(this, window, cx);
        return div()
            .id("chat-start")
            .relative()
            .top(px((1. - reveal) * 14.))
            .opacity(reveal)
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .bg(p.card)
            .children(connection_problem)
            .children(coordination)
            .children(trust_notice(this, cx))
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
    let messages = conversation(this, window, cx);
    let composer = composer(this, window, cx);

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(p.card)
        .children(connection_problem)
        .children(coordination)
        .children(trust_notice(this, cx))
        .child(messages)
        .child(composer)
        .into_any_element()
}

fn trust_notice(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> Option<AnyElement> {
    if this.projects.is_empty() || this.project().trusted {
        return None;
    }
    let p = palette(cx);
    Some(
        div()
            .w_full()
            .px(px(22.))
            .py(px(12.))
            .flex()
            .justify_center()
            .flex_shrink_0()
            .child(
                div()
                    .w_full()
                    .max_w(px(824.))
                    .p(px(12.))
                    .rounded(px(dunlin_ui::radii::MESSAGE))
                    .border_1()
                    .border_color(p.amber.opacity(0.35))
                    .bg(p.amber_soft)
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        Icon::new(IconName::ShieldAlert)
                            .size(px(14.))
                            .text_color(p.amber),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.fg)
                                    .child("This project is not trusted"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(p.muted_fg)
                                    .child("Agents, Git operations and project runtimes stay stopped until you trust this folder."),
                            ),
                    )
                    .child(
                        dunlin_ui::action_button("trust-project-banner")
                            .label("Trust project")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_project_trust(cx))),
                    ),
            )
            .into_any_element(),
    )
}

fn connection_problem(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> Option<AnyElement> {
    let command = this.harness.missing_command.as_ref()?;
    let p = palette(cx);
    let can_use_opencode = this.session_harness() != "OpenCode"
        && this.harness_unavailable_reason("OpenCode").is_none();
    Some(div().w_full().px(px(22.)).py(px(12.)).flex().justify_center().flex_shrink_0().child(
        div().w_full().max_w(px(824.)).p(px(12.)).rounded(px(dunlin_ui::radii::MESSAGE))
            .border_1().border_color(p.amber.opacity(0.35)).bg(p.amber_soft)
            .flex().flex_col().gap(px(8.))
            .child(div().text_size(px(13.)).text_color(p.amber).child(format!("{} needs an ACP adapter", this.session_harness())))
            .child(div().text_size(px(12.)).text_color(p.muted_fg).child(
                "The required ACP command is not installed. No connection is being retried. Your existing conversations are preserved."
            ))
            .child(div().text_size(px(11.5)).font_family(dunlin_ui::theme::code_font()).child(command.clone()))
            .child(div().flex().flex_wrap().gap(px(8.))
                .when(can_use_opencode, |view| view.child(
                    dunlin_ui::action_button("missing-adapter-opencode").primary().label("Start with OpenCode")
                        .on_click(cx.listener(|this, _, _, cx| this.select_chat_harness("OpenCode", cx)))))
                .child(dunlin_ui::action_button("missing-adapter-settings").label("Harness settings")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings_open = true;
                        this.settings_tab = SettingsTab::Harnesses;
                        cx.notify();
                    })))
                .child(dunlin_ui::action_button("missing-adapter-retry").label("Check again")
                    .on_click(cx.listener(|this, _, _, cx| this.connect_harness(cx)))))
    ).into_any_element())
}

fn conversation(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let session = this.session().id.clone();
    let following = this.chat_scroll.prepare(&session);
    let reveal = this.chat_scroll.reveal.take();
    let final_only = this.final_answer_only && this.expanded_tool.is_none();
    let scroll_probe = this.chat_scroll.handle.clone();
    // Normal presentation preserves indexes. Bound even legacy coalescing to
    // the visible window; final-only filtering needs the full turn context.
    let source_start = if final_only {
        0
    } else {
        crate::chat_presentation::window_start(&this.harness.transcript, this.chat_visible_items)
    };
    let items = crate::chat_presentation::project(
        &this.harness.transcript[source_start..],
        final_only,
        this.harness.running,
        this.stream_responses,
    );
    let running = this.harness.running;
    let status = this.harness.status.clone();
    let reasoning_open = this.reasoning_open;
    let expanded = this.expanded_tool;
    let waiting_for_permission = this
        .harness
        .transcript
        .iter()
        .any(|item| matches!(item, ChatItem::Permission { resolved: None, .. }));

    let mut inner = div()
        .w_full()
        .flex_shrink_0()
        .min_w(px(0.))
        .pb(px(12.))
        .flex()
        .flex_col()
        .gap(px(18.));
    let first_visible = crate::chat_presentation::window_start(&items, this.chat_visible_items);
    let earlier_count = source_start + first_visible;

    if earlier_count > 0 {
        inner = inner.child(
            div().w_full().flex().justify_center().child(
                dunlin_ui::button("chat-show-earlier")
                    .ghost()
                    .small()
                    .icon(IconName::ChevronUp)
                    .label(format!(
                        "Show {} earlier {}",
                        earlier_count,
                        if earlier_count == 1 {
                            "message"
                        } else {
                            "messages"
                        }
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.chat_scroll.pause_following();
                        this.chat_visible_items = this
                            .chat_visible_items
                            .saturating_add(crate::app::CHAT_WINDOW);
                        cx.notify();
                    })),
            ),
        );
    }

    if items.is_empty() {
        inner = inner.child(empty_state(&status, this.project().is_remote(), cx, &p));
    } else {
        // Final-only presentation removes tool/thought rows, not user turns.
        // Keep retry/selection identities tied to the original transcript.
        let skipped_users = items[..first_visible]
            .iter()
            .filter(|item| matches!(item, ChatItem::User { .. }))
            .count();
        let mut user_indices = this
            .harness
            .transcript
            .iter()
            .enumerate()
            .skip(source_start)
            .filter(|(_, item)| matches!(item, ChatItem::User { .. }))
            .map(|(index, _)| index)
            .skip(skipped_users);
        let mut index = first_visible;
        while index < items.len() {
            if let ChatItem::User { text, at_ms, .. } = &items[index] {
                let source_index = user_indices
                    .next()
                    .expect("presentation preserves user turns");
                inner = inner.child(user_message(
                    text,
                    UserMessageOptions {
                        index: source_index,
                        time: this
                            .chat_timestamps
                            .then(|| crate::chat_timing::local_time(*at_ms))
                            .flatten(),
                        failure: crate::chat_presentation::prompt_failure(
                            &this.harness.transcript,
                            source_index,
                        ),
                        retry_enabled: !this.chat_controls_busy()
                            && !this.harness.has_queued_prompts()
                            && !this.group_cleanup.busy,
                        session: &session,
                    },
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
                    timing.clone()
                } else {
                    None
                }
            });
            let caption = crate::chat_timing::agent_caption(
                &items[start..index],
                timing.clone(),
                this.chat_timestamps,
                this.chat_duration,
            );
            let speaker = timing.and_then(|timing| timing.speaker).unwrap_or_else(|| {
                crate::harness::Speaker {
                    harness: this.session_harness(),
                    model: None,
                }
            });
            let mut group =
                div()
                    .id(("chat-agent-group", source_start + start))
                    .child(agent_group(
                        &items[start..index],
                        source_start + start,
                        AgentGroupOptions {
                            reasoning_open,
                            expanded,
                            session: &session,
                            reduce_motion: this.reduce_motion || cx.reduce_motion(),
                            caption,
                            logo: this.chat_logo,
                            speaker,
                        },
                        window,
                        cx,
                        &p,
                    ));
            if reveal.is_some_and(|target| {
                (source_start + start..source_start + index).contains(&target)
            }) {
                let anchor = ScrollAnchor::for_handle(this.chat_scroll.handle.clone());
                anchor.scroll_to(window, cx);
                group = group.anchor_scroll(Some(anchor));
            }
            inner = inner.child(group);
        }
    }

    if running && !waiting_for_permission {
        inner = inner.child(running_card(
            this.final_answer_only,
            this.stream_responses,
            this.reduce_motion || cx.reduce_motion(),
            &p,
        ));
    }

    #[cfg(debug_assertions)]
    let inner = inner
        .relative()
        .children(crate::workspace_ui_smoke::probe("conversation"));
    let scroll_session = this.session().id.clone();
    let stream = div()
        .id("chat-stream")
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .overflow_y_scroll()
        .track_scroll(&this.chat_scroll.handle)
        .on_scroll(cx.listener(move |this, event: &ScrollEvent, window, cx| {
            let delta = event.delta.pixel_delta(px(16.)).y;
            let current = this.session().id == scroll_session;
            if delta > px(0.) {
                let changed = if current {
                    this.chat_scroll.pause_following()
                } else if let Some(scroll) = this.workbench.chat_scrolls.get_mut(&scroll_session) {
                    scroll.pause_following()
                } else {
                    false
                };
                if changed {
                    cx.notify();
                }
                return;
            }
            if delta == px(0.) {
                return;
            }
            // Read the resulting offset after GPUI's default wheel handler.
            // Sampling during normal renders would confuse growing content
            // with the reader deliberately scrolling away from the live edge.
            let app = cx.entity().downgrade();
            let session = scroll_session.clone();
            window.defer(cx, move |_, cx| {
                let _ = app.update(cx, |this, cx| {
                    let changed = if this.session().id == session {
                        this.chat_scroll.user_scrolled()
                    } else if let Some(scroll) = this.workbench.chat_scrolls.get_mut(&session) {
                        scroll.user_scrolled()
                    } else {
                        false
                    };
                    if changed {
                        cx.notify();
                    }
                });
            });
        }))
        .px(px(22.))
        .py(px(18.))
        .flex()
        .flex_col()
        .items_center()
        .child(inner);
    let stream = text::auto_copy(stream, &session, window, cx);

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
                        dunlin_ui::button("chat-jump-latest")
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
                .rounded(px(dunlin_ui::radii::CARD))
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
    action_row(format!("starter:{title}"), title.to_owned())
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
                .rounded(px(dunlin_ui::radii::CONTROL))
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
    reduce_motion: bool,
    caption: Option<String>,
    logo: bool,
    speaker: crate::harness::Speaker,
}

fn agent_group(
    items: &[ChatItem],
    base: usize,
    options: AgentGroupOptions<'_>,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let mut body = div()
        .w_full()
        .min_w(px(0.))
        .pl(px(if options.logo { 27. } else { 0. }))
        .flex()
        .flex_col()
        .gap(px(14.));
    let mut offset = 0;
    for section in items.chunk_by(|a, b| work::is_work(a) == work::is_work(b)) {
        let index = base + offset;
        if work::is_work(&section[0]) {
            let (card, open) = work::card(section, index, &options, window, cx, p);
            body = body.child(card.when(open, |card| {
                card.child(
                    div()
                        .px(px(10.))
                        .pb(px(9.))
                        .child(agent_items(section, index, &options, cx, p)),
                )
            }));
        } else {
            body = body.child(agent_items(section, index, &options, cx, p));
        }
        offset += section.len();
    }

    div()
        .w_full()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(px(9.))
        .child(agent_header(
            options.logo,
            &options.speaker,
            options.caption,
            p,
        ))
        .child(body)
        .into_any_element()
}

fn agent_items(
    items: &[ChatItem],
    base: usize,
    options: &AgentGroupOptions<'_>,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let reasoning_open = options.reasoning_open;
    let expanded = options.expanded;
    let session = options.session;
    let reduce_motion = options.reduce_motion;
    let mut body = div().w_full().min_w(px(0.)).flex().flex_col().gap(px(14.));

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
                let mut text_block = div()
                    .id(("agent-text", index))
                    .relative()
                    .children(crate::gui_probe::marker("agent-reply"))
                    .children({
                        #[cfg(debug_assertions)]
                        {
                            crate::workspace_ui_smoke::probe("chat-reading-text")
                        }
                        #[cfg(not(debug_assertions))]
                        {
                            None::<AnyElement>
                        }
                    })
                    .pr(px(32.))
                    .text_size(crate::workspace_text::size(13., cx))
                    .line_height(relative(1.62))
                    .text_color(p.fg.opacity(0.92))
                    .child(
                        markdown_text(format!("response:{session}:{index}"), text.clone())
                            .text_size(crate::workspace_text::size(13., cx)),
                    );
                if !*streaming {
                    text_block = text_block.child(
                        icon_button(("agent-copy", index), IconName::Copy, "Copy response")
                            .absolute()
                            .top(px(0.))
                            .right(px(0.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                                this.set_notice("Copied the response", cx);
                            })),
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
                body = body.child(tool_card(
                    view,
                    *duration_ms,
                    ToolCardOptions {
                        index,
                        expanded,
                        session,
                        reduce_motion,
                    },
                    cx,
                    p,
                ));
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
                        .rounded(px(dunlin_ui::radii::PANEL))
                        .border_1()
                        .border_color(p.border)
                        .p(px(8.))
                        .child(
                            dunlin_ui::button(("child-details", index))
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

    body.into_any_element()
}

pub(crate) fn markdown_text(id: String, text: String) -> TextView {
    TextView::markdown(id, text)
        .scrollable(false)
        .selectable(true)
        .on_link_click(|url, _, _, cx| {
            if response_link(url) {
                cx.open_url(url);
            }
        })
}

fn response_link(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "mailto"))
}

#[cfg(test)]
mod link_tests {
    #[test]
    fn response_links_allow_web_and_email_but_not_local_files_or_executable_schemes() {
        for value in [
            "https://example.com/docs?q=1#section",
            "http://localhost:3000",
            "mailto:person@example.com",
        ] {
            assert!(super::response_link(value));
        }
        for value in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,hello",
            "relative/path",
        ] {
            assert!(!super::response_link(value));
        }
    }
}

struct UserMessageOptions<'a> {
    index: usize,
    time: Option<String>,
    failure: Option<&'a str>,
    retry_enabled: bool,
    session: &'a str,
}

fn user_message(
    text: &str,
    options: UserMessageOptions<'_>,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let UserMessageOptions {
        index,
        time,
        failure,
        retry_enabled,
        session,
    } = options;
    let failed = failure.is_some();

    div()
        .id(("user-message", index))
        .w_full()
        .flex()
        .justify_end()
        .child(
            div()
                .relative()
                .children(crate::gui_probe::marker_named(format!(
                    "user-bubble-{index}"
                )))
                .when(failed, |bubble| {
                    bubble.children(crate::gui_probe::marker_named(format!(
                        "failed-user-{index}"
                    )))
                })
                .max_w(px(620.))
                .min_w(px(0.))
                .px(px(13.))
                .py(px(10.))
                .rounded(px(dunlin_ui::radii::GRAPH_NODE))
                .bg(if failed { p.red_soft } else { p.muted })
                .border_1()
                .border_color(if failed { p.red.opacity(0.5) } else { p.border })
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(
                    div()
                        .relative()
                        .children(crate::gui_probe::marker_named(format!("user-text-{index}")))
                        .min_w(px(0.))
                        .text_size(crate::workspace_text::size(13., cx))
                        .line_height(relative(1.6))
                        .text_color(if failed { p.red } else { p.fg })
                        .child(
                            text::plain(format!("user-text:{session}:{index}"), text)
                                .text_size(crate::workspace_text::size(13., cx)),
                        ),
                )
                .when(time.is_some() || failed, |bubble| {
                    bubble.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .when_some(time, |row, time| {
                                row.child(
                                    div()
                                        .text_size(px(11.0))
                                        .font_family(dunlin_ui::theme::code_font())
                                        .text_color(p.muted_fg.opacity(0.75))
                                        .child(time),
                                )
                            })
                            .when(failed, |row| {
                                row.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(p.red)
                                        .child("Request failed"),
                                )
                            })
                            .child(div().flex_1())
                            .when(failed, |row| {
                                row.child(crate::gui_probe::target(
                                    format!("user-retry-{index}"),
                                    icon_button(
                                        ("user-retry", index),
                                        IconName::RotateCcw,
                                        "Retry failed message",
                                    )
                                    .disabled(!retry_enabled)
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.retry_prompt(index, window, cx);
                                        },
                                    )),
                                ))
                            }),
                    )
                }),
        )
        .into_any_element()
}

fn agent_avatar(harness: &str, p: &Palette) -> AnyElement {
    crate::harness_brand::logo(harness, 20., p.fg)
}

fn agent_header(
    logo: bool,
    speaker: &crate::harness::Speaker,
    caption: Option<String>,
    p: &Palette,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(7.))
        .when(logo, |view| view.child(agent_avatar(&speaker.harness, p)))
        .child(
            div()
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.fg)
                .child(crate::harness_brand::signature(
                    &speaker.harness,
                    speaker.model.as_deref(),
                )),
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

    let mut card = card_surface(*p)
        .border_color(p.border)
        .bg(p.subtle)
        .overflow_hidden()
        .child(
            card_action_row("reasoning-card", "Toggle thought details", open)
                .toggled(open)
                .icon(IconName::Brain)
                .label("Thought")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.reasoning_open = !this.reasoning_open;
                    cx.notify();
                }))
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

struct ToolCardOptions<'a> {
    index: usize,
    expanded: Option<usize>,
    session: &'a str,
    reduce_motion: bool,
}

// Every tool row reserves the same columns, including absent metadata. Only
// the path/summary between the name and these columns may grow or shrink.
fn tool_column(width: f32) -> Div {
    div()
        .w(px(width))
        .min_w(px(width))
        .max_w(px(width))
        .flex_shrink_0()
        .overflow_hidden()
}

fn tool_card(
    tool: &ToolCallView,
    duration_ms: u64,
    options: ToolCardOptions<'_>,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let ToolCardOptions {
        index,
        expanded,
        session,
        reduce_motion,
    } = options;
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
    let has_changes = additions > 0 || deletions > 0;
    let diff = tool.diff.clone();

    let mut card = card_surface(*p).overflow_hidden().child(
        card_action_row(
            ("tool-card", index),
            format!("Toggle tool details: {title}"),
            is_open,
        )
        .toggled(is_open)
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
                .rounded(px(dunlin_ui::radii::ROW))
                .bg(p.muted)
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(icon_name).size(px(12.)).text_color(p.muted_fg)),
        )
        .child(
            tool_column(80.)
                .truncate()
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
        .child(
            tool_column(44.)
                .truncate()
                .text_right()
                .text_size(px(11.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.green)
                .child(if has_changes {
                    format!("+{additions}")
                } else {
                    String::new()
                }),
        )
        .child(
            tool_column(44.)
                .truncate()
                .text_right()
                .text_size(px(11.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.red)
                .child(if has_changes {
                    format!("−{deletions}")
                } else {
                    String::new()
                }),
        )
        .child(
            tool_column(78.)
                .h(px(16.))
                .px(px(5.))
                .rounded_full()
                .bg(status_soft)
                .flex()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .when(status == ToolStatus::Running, |this| {
                    this.child(working_icon(8.5, status_color, reduce_motion))
                })
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(status_color)
                        .child(
                            MorphText::new(
                                format!("tool-status:{}", tool.id),
                                status.label().to_uppercase(),
                            )
                            .scope(session.to_string())
                            .reduce_motion(reduce_motion),
                        ),
                ),
        )
        .child(
            tool_column(64.)
                .truncate()
                .text_right()
                .text_size(px(11.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child(if duration_ms > 0 {
                    format_ms(duration_ms)
                } else {
                    String::new()
                }),
        )
        .child(
            tool_column(12.)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(if is_open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(px(11.5))
                    .text_color(p.muted_fg),
                ),
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
                    .rounded(px(dunlin_ui::radii::ICON))
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

    code_surface(*p)
        .child(
            code_header(*p)
                .child(
                    div()
                        .text_size(px(10.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child("DIFF"),
                )
                .child(div().flex_1())
                .child(
                    icon_button(("tool-diff-copy", index), IconName::Copy, "Copy diff").on_click(
                        cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
                            this.set_notice("Copied the diff", cx);
                        }),
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
    let mut card = card_surface(*p)
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

    let mut card = card_surface(*p)
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
            buttons = buttons.child(crate::gui_probe::target(
                format!("permission-{}", option.id),
                dunlin_ui::action_button(format!("permission:{request_id}:{}", option.id))
                    .label(option.label.clone())
                    .when(primary, |button| button.primary())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.resolve_permission(
                            &request,
                            Some(option_id.clone()),
                            label.clone(),
                            cx,
                        );
                    })),
            ));
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
        .w_full()
        .min_w(px(0.))
        .relative()
        .children(crate::gui_probe::marker("chat-error-card"))
        .tooltip(move |window, cx| {
            Tooltip::new(details.clone())
                .max_w(px(600.))
                .build(window, cx)
        })
        .rounded(px(dunlin_ui::radii::MESSAGE))
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
                .flex_shrink_0()
                .text_color(p.red),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .relative()
                .children(crate::gui_probe::marker("chat-error-text"))
                .text_size(px(12.0))
                .line_height(relative(1.5))
                .text_color(p.red)
                .child(
                    text::plain(format!("chat-error-text:{index}"), display)
                        .w_full()
                        .text_size(px(12.)),
                )
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

fn working_icon(size: f32, color: Hsla, reduce_motion: bool) -> AnyElement {
    if reduce_motion {
        Icon::new(IconName::LoaderCircle)
            .size(px(size))
            .text_color(color)
            .into_any_element()
    } else {
        dunlin_ui::component::spinner::Spinner::new()
            .icon(IconName::LoaderCircle)
            .with_size(px(size))
            .color(color)
            .ease(|progress| progress)
            .into_any_element()
    }
}

fn running_card(final_only: bool, stream: bool, reduce_motion: bool, p: &Palette) -> AnyElement {
    div()
        .rounded(px(dunlin_ui::radii::MESSAGE))
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
                .child(working_icon(12.5, p.blue, reduce_motion))
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
                    "Response text appears as it arrives; agent work stays in Graph and file edits in Timeline"
                } else {
                    "tool calls and reasoning will appear inline"
                }),
        )
        .into_any_element()
}

fn composer(this: &mut DunlinApp, window: &mut Window, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let running = this.harness.running;
    let has_text = !this.composer.read(cx).value().trim().is_empty();
    let queued = this.harness.queued_prompts();
    let queue_open = this.queue_open;
    let placeholder = format!(
        "Ask {} to build, fix, or explain…",
        chat_controls::harness_label(this)
    );
    if this.composer.read(cx).presentation().placeholder().as_ref() != placeholder {
        this.composer.update(cx, |state, cx| {
            state.set_placeholder(placeholder, window, cx)
        });
    }

    let input = composer_input(&this.composer)
        .aria_label("Message to agent")
        .text_size(crate::workspace_text::size(13., cx))
        .h(px(72.));
    let harness = chat_controls::harness(this, cx);
    let mode = chat_controls::mode(this, cx);
    let model = chat_controls::model(this, cx);

    let card = SurfaceGeometry::COMPOSER
        .paint(p.subtle, p.border)
        .w_full()
        .border_color(
            if this.composer.read(cx).focus_handle(cx).is_focused(window) {
                p.blue
            } else {
                p.border
            },
        )
        .shadow_sm()
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
                // GPUI overflow masks are rectangular; round this painted child too.
                .rounded_t(px(SurfaceGeometry::COMPOSER.inner_radius()))
                .when(this.git_history.cancel_check.is_some(), |bar| {
                    bar.child(
                        dunlin_ui::action_button("composer-stop-check")
                            .label("Stop check")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_check(cx))),
                    )
                })
                .child(harness)
                .children(chat_controls::provider_controls(this, cx))
                .child(div().flex_1())
                .when(!queued.is_empty(), |bar| {
                    bar.child(crate::gui_probe::target(
                        "composer-queue",
                        dunlin_ui::button("composer-queue")
                            .warning()
                            .outline()
                            .icon(IconName::Clock3)
                            .child(
                                MorphText::new("composer-queue-count", queued.len().to_string())
                                    .numeric()
                                    .scope(this.session().id.clone())
                                    .reduce_motion(this.reduce_motion),
                            )
                            .child("queued")
                            .accessibility_label(format!(
                                "Toggle prompt queue: {} queued",
                                queued.len()
                            ))
                            .selected(queue_open)
                            .toggled(queue_open)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.queue_open = !this.queue_open;
                                cx.notify();
                            })),
                    ))
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
                .child(chat_controls::options(this, cx))
                .child(mode)
                .children(chat_controls::model_options(this, cx))
                .child(div().flex_1())
                .child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(model)
                        .child(crate::gui_probe::target(
                            "composer-send",
                            icon_button(
                                "composer-send",
                                if running {
                                    IconName::CircleStop
                                } else {
                                    IconName::ArrowUp
                                },
                                if running {
                                    "Stop response"
                                } else {
                                    "Send prompt"
                                },
                            )
                            .tooltip(if running {
                                "Stop response".into()
                            } else {
                                this.shortcut_label("Send prompt", "Enter")
                            })
                            .primary()
                            .when(running, |button| button.info())
                            .disabled(!running && !has_text)
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    if this.harness.running {
                                        this.cancel_turn(cx);
                                    } else {
                                        this.send_prompt(window, cx);
                                    }
                                },
                            )),
                        )),
                ),
        );

    #[cfg(debug_assertions)]
    let card = card
        .relative()
        .children(crate::workspace_ui_smoke::probe("composer"));
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
                .when(queue_open && !queued.is_empty(), |view| {
                    view.child(queue_panel(this, &queued, cx, &p))
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

fn queue_panel(
    this: &DunlinApp,
    queued: &[QueuedPrompt],
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
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
                .rounded(px(dunlin_ui::radii::ROW))
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
                    icon_button(("queued-up", index), IconName::ChevronUp, "Move prompt up")
                        .disabled(first)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.reorder_queued_prompt(id, -1, cx);
                        })),
                )
                .child(
                    icon_button(
                        ("queued-down", index),
                        IconName::ChevronDown,
                        "Move prompt down",
                    )
                    .disabled(last)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.reorder_queued_prompt(id, 1, cx);
                    })),
                )
                .child(
                    icon_button(
                        ("queued-edit", index),
                        IconName::Pencil,
                        "Edit queued prompt",
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_queued_prompt(id, window, cx);
                    })),
                )
                .child(
                    icon_button(
                        ("queued-remove", index),
                        IconName::X,
                        "Remove queued prompt",
                    )
                    .danger()
                    .outline()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.remove_queued_prompt(id, cx);
                    })),
                ),
        );
    }

    card_surface(*p)
        .w_full()
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
                        .child(
                            MorphText::new("queue-panel-count", queued.len().to_string())
                                .numeric()
                                .scope(this.session().id.clone())
                                .reduce_motion(this.reduce_motion),
                        ),
                )
                .child(div().flex_1())
                .child(crate::gui_probe::target(
                    "queue-clear",
                    dunlin_ui::action_button("queue-clear")
                        .danger()
                        .outline()
                        .label("Clear")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.clear_queued_prompts(cx);
                        })),
                ))
                .child(
                    icon_button("queue-close", IconName::X, "Close prompt queue").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.queue_open = false;
                            cx.notify();
                        }),
                    ),
                ),
        )
        .child(rows)
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
    use super::{repeated_errors, tool_column};
    use crate::harness::ChatItem;
    use dunlin_ui::*;

    #[test]
    fn tool_columns_keep_fixed_widths_even_with_missing_or_long_values() {
        for width in [80., 44., 78., 64., 12.] {
            for value in ["", "DONE", "RUNNING", "+123456789", "12345m 59s"] {
                let mut column = tool_column(width).child(value);
                let style = column.style();
                assert_eq!(style.size.width, Some(px(width).into()));
                assert_eq!(style.min_size.width, Some(px(width).into()));
                assert_eq!(style.max_size.width, Some(px(width).into()));
                assert_eq!(style.flex_shrink, Some(0.));
            }
        }
    }

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
