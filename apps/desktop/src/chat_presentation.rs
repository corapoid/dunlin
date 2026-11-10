//! Presentation filters never alter the stored transcript or permission flow.
use crate::harness::ChatItem;
use dunlin_agent::ToolCallView;
use dunlin_domain::{ToolKind, ToolStatus};
use std::borrow::Cow;

pub(crate) fn project(
    items: &[ChatItem],
    final_only: bool,
    running: bool,
    stream: bool,
) -> Cow<'_, [ChatItem]> {
    if !final_only {
        if (!running || stream) && !needs_coalescing(items) {
            return Cow::Borrowed(items);
        }
        let mut visible = coalesce_legacy(items);
        if running && !stream {
            let start = visible
                .iter()
                .rposition(|item| matches!(item, ChatItem::User { .. }))
                .unwrap_or(0);
            for item in &mut visible[start..] {
                if let ChatItem::AgentText { text, .. } = item {
                    text.clear();
                }
            }
        }
        return Cow::Owned(visible);
    }
    let mut output = Vec::new();
    let mut start = 0;
    while start < items.len() {
        let end = items
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, item)| matches!(item, ChatItem::User { .. }))
            .map(|(i, _)| i)
            .unwrap_or(items.len());
        let turn = &items[start..end];
        if let Some(ChatItem::User { .. }) = turn.first() {
            output.push(turn[0].clone());
        }
        output.extend(
            turn.iter()
                .filter(|item| {
                    matches!(
                        item,
                        ChatItem::Error { .. } | ChatItem::Permission { resolved: None, .. }
                    )
                })
                .cloned(),
        );
        if stream || !(running && end == items.len()) {
            let after_tool = turn
                .iter()
                .rposition(|item| matches!(item, ChatItem::Tool { .. }))
                .map(|i| i + 1)
                .unwrap_or(0);
            let candidates = &turn[after_tool..];
            let last = candidates.iter().rev().find_map(|item| {
                if let ChatItem::AgentText { message_id, .. } = item {
                    Some(message_id)
                } else {
                    None
                }
            });
            let mut text = String::new();
            let mut at = 0;
            for item in candidates {
                if let ChatItem::AgentText {
                    text: part,
                    at_ms,
                    message_id,
                    ..
                } = item
                    && last.is_some_and(|last| last.is_none() || last == message_id)
                {
                    if at == 0 {
                        at = *at_ms;
                    }
                    text.push_str(part);
                }
            }
            if !text.trim().is_empty() {
                output.push(ChatItem::AgentText {
                    text,
                    streaming: running && end == items.len(),
                    message_id: last.cloned().flatten(),
                    at_ms: at,
                });
            }
        }
        start = end;
    }
    Cow::Owned(output)
}

fn needs_coalescing(items: &[ChatItem]) -> bool {
    let mut legacy = false;
    for item in items {
        match item {
            ChatItem::AgentText {
                message_id: None, ..
            } => {
                if legacy {
                    return true;
                }
                legacy = true;
            }
            ChatItem::Thought { .. } => {}
            _ => legacy = false,
        }
    }
    false
}

pub(crate) fn has_reasoning(items: &[ChatItem]) -> bool {
    items.iter().any(|item| match item {
        ChatItem::Thought { text, .. } => !text.trim().is_empty(),
        ChatItem::Child { items, .. } => has_reasoning(items),
        _ => false,
    })
}

/// Older transcripts have no per-request failure metadata. Recognize only
/// explicit prompt/transport failures, never an unrelated model/config error.
pub(crate) fn prompt_failure(items: &[ChatItem], index: usize) -> Option<&str> {
    let ChatItem::User { timing, .. } = items.get(index)? else {
        return None;
    };
    if let Some(failure) = timing.as_ref().and_then(|timing| timing.failure.as_deref()) {
        return Some(failure);
    }
    items[index + 1..]
        .iter()
        .take_while(|item| !matches!(item, ChatItem::User { .. }))
        .find_map(|item| match item {
            ChatItem::Error { message, .. }
                if message.starts_with("Prompt failed:")
                    || message.starts_with("Could not send the prompt:") =>
            {
                Some(message.as_str())
            }
            _ => None,
        })
}

/// First visible item for the chat window. Prefer starting on a user turn when
/// one is close to the cutoff, otherwise the window starts mid-turn so long
/// tool runs still load incrementally.
pub(crate) fn window_start(items: &[ChatItem], window: usize) -> usize {
    let cutoff = items.len().saturating_sub(window.max(1));
    if cutoff == 0 {
        return 0;
    }
    match items[..=cutoff]
        .iter()
        .rposition(|item| matches!(item, ChatItem::User { .. }))
    {
        Some(start) if cutoff - start <= window => start,
        _ => cutoff,
    }
}

/// Older OpenCode imports stored tool calls as `[Tool · name]` text. Rewriting
/// them in memory keeps legacy transcripts from coalescing megabytes of output
/// into one Markdown block and gives them the normal expandable tool card.
pub(crate) fn normalize_legacy(items: &mut [ChatItem]) {
    let mut counter = 0;
    normalize_legacy_items(items, &mut counter);
}

fn normalize_legacy_items(items: &mut [ChatItem], counter: &mut usize) {
    for item in items {
        match item {
            ChatItem::AgentText {
                text,
                message_id: None,
                at_ms,
                ..
            } => {
                if let Some(tool) = legacy_tool_item(text, *at_ms, *counter) {
                    *counter += 1;
                    *item = tool;
                }
            }
            ChatItem::Child { items, .. } => normalize_legacy_items(items, counter),
            _ => {}
        }
    }
}

fn legacy_tool_item(text: &str, at: u64, sequence: usize) -> Option<ChatItem> {
    let rest = text.strip_prefix("[Tool · ")?;
    let (name, body) = rest.split_once(']')?;
    let name = name.trim();
    if name.is_empty() || name.len() > 64 {
        return None;
    }
    let mut title = String::new();
    let mut status = ToolStatus::Done;
    let mut detail = String::new();
    for line in body.lines() {
        let line = line.trim_end();
        if let Some(value) = line.strip_prefix("Status: ") {
            status = match value.trim() {
                "running" => ToolStatus::Running,
                "pending" | "waiting" => ToolStatus::Waiting,
                "error" | "failed" => ToolStatus::Failed,
                _ => ToolStatus::Done,
            };
        } else if let Some(value) = line.strip_prefix("Input: ") {
            if !detail.is_empty() {
                detail.push('\n');
            }
            detail.push_str(value);
        } else if let Some(value) = line.strip_prefix("Output: ") {
            if !detail.is_empty() {
                detail.push('\n');
            }
            detail.push_str(value);
        } else if title.is_empty() && !line.trim().is_empty() {
            title = line.trim().to_string();
        }
    }
    if !title.is_empty() {
        detail.insert_str(0, &format!("{title}\n"));
    }
    let detail = detail.chars().take(4096).collect();
    Some(ChatItem::Tool {
        view: ToolCallView {
            id: format!("legacy-tool-{at}-{sequence}"),
            title: name.to_string(),
            kind: legacy_tool_kind(name),
            status,
            detail,
            locations: Vec::new(),
            additions: 0,
            deletions: 0,
            diff: None,
            file_changes: Vec::new(),
        },
        duration_ms: 0,
        at_ms: at,
    })
}

fn legacy_tool_kind(name: &str) -> ToolKind {
    match name.to_ascii_lowercase().as_str() {
        "shell" | "bash" | "exec" => ToolKind::Shell,
        "read" | "view" => ToolKind::Read,
        "grep" | "glob" | "search" | "find" => ToolKind::Search,
        "edit" | "write" | "patch" | "apply_patch" => ToolKind::Edit,
        "webfetch" | "websearch" | "fetch" => ToolKind::Web,
        "test" => ToolKind::Test,
        "task" => ToolKind::Task,
        "plan" | "todowrite" | "todoread" => ToolKind::Plan,
        _ => ToolKind::Task,
    }
}

/// Older saved transcripts have no message IDs. Join text fragments separated
/// only by reasoning, but never cross a tool call, turn or child-session boundary.
fn coalesce_legacy(items: &[ChatItem]) -> Vec<ChatItem> {
    let mut output: Vec<ChatItem> = Vec::new();
    let mut message: Option<usize> = None;
    for item in items {
        match item {
            ChatItem::AgentText {
                text,
                streaming,
                message_id: None,
                ..
            } => {
                if let Some(index) = message
                    && let ChatItem::AgentText {
                        text: body,
                        streaming: active,
                        ..
                    } = &mut output[index]
                {
                    body.push_str(text);
                    *active = *streaming;
                    let mut hidden = item.clone();
                    if let ChatItem::AgentText { text, .. } = &mut hidden {
                        text.clear();
                    }
                    output.push(hidden);
                    continue;
                }
                message = Some(output.len());
            }
            ChatItem::Thought { .. } => {}
            _ => message = None,
        }
        output.push(item.clone());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn retry_is_request_scoped_and_legacy_transport_errors_remain_retryable() {
        let user = || ChatItem::User {
            text: "Request".into(),
            timing: None,
            at_ms: 1,
        };
        let error = |message: &str| ChatItem::Error {
            message: message.into(),
            at_ms: 2,
        };
        assert!(prompt_failure(&[user(), error("Model selection denied")], 0).is_none());
        let history = [
            user(),
            error("Prompt failed: provider declined the request"),
            user(),
            message("Success"),
        ];
        assert_eq!(
            prompt_failure(&history, 0),
            Some("Prompt failed: provider declined the request")
        );
        assert!(prompt_failure(&history, 2).is_none());
        assert!(prompt_failure(&history, 1).is_none());
        assert!(prompt_failure(&history, 99).is_none());
    }

    #[test]
    fn ordinary_chat_borrows_even_large_tool_results_instead_of_cloning_them_on_scroll() {
        let items = vec![
            tool(),
            ChatItem::AgentText {
                text: "x".repeat(1_000_000),
                streaming: false,
                message_id: Some("answer".into()),
                at_ms: 1,
            },
        ];
        assert!(matches!(
            project(&items, false, false, true),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            project(&items, false, true, true),
            Cow::Borrowed(_)
        ));
        assert!(matches!(project(&items, false, true, false), Cow::Owned(_)));
        assert!(matches!(
            project(
                &[message("one"), thought(), message("two")],
                false,
                false,
                true
            ),
            Cow::Owned(_)
        ));
    }

    #[test]
    fn legacy_projection_can_be_windowed_without_losing_absolute_tool_indexes() {
        let mut history = Vec::new();
        for turn in 0..500 {
            history.push(ChatItem::User {
                text: turn.to_string(),
                at_ms: 1,
                timing: None,
            });
            history.extend([message("one"), thought(), message("two"), tool()]);
        }
        let start = window_start(&history, 120);
        let visible = project(&history[start..], false, false, true);
        assert!(start > 2000 && visible.len() <= 240);
        assert_eq!(visible.len(), history.len() - start);
        for (index, item) in visible.iter().enumerate() {
            if let ChatItem::Tool { .. } = item {
                assert!(matches!(history[start + index], ChatItem::Tool { .. }));
            }
        }
    }

    #[test]
    fn reasoning_availability_requires_shared_nonempty_thoughts() {
        assert!(!has_reasoning(&[message("Answer")]));
        assert!(has_reasoning(&[thought()]));
        let child = ChatItem::Child {
            id: "child".into(),
            parent_id: "parent".into(),
            title: None,
            items: vec![thought()],
            at_ms: 1,
        };
        assert!(has_reasoning(&[child]));
    }

    fn message(text: &str) -> ChatItem {
        ChatItem::AgentText {
            text: text.into(),
            streaming: false,
            message_id: None,
            at_ms: 1,
        }
    }
    fn thought() -> ChatItem {
        ChatItem::Thought {
            text: "internal".into(),
            streaming: false,
            message_id: None,
            at_ms: 2,
        }
    }
    fn tool() -> ChatItem {
        serde_json::from_value(json!({"Tool":{"view":{"id":"tool","title":"Search","kind":"Web","status":"Done","detail":"raw output","locations":[],"additions":0,"deletions":0,"diff":null},"duration_ms":5}})).unwrap()
    }

    #[test]
    fn quiet_chat_hides_workflow_and_waits_for_a_completed_root_answer() {
        let user = ChatItem::User {
            text: "Question".into(),
            timing: None,
            at_ms: 1,
        };
        let permission = ChatItem::Permission {
            request_id: "ask".into(),
            title: "Permission".into(),
            options: Vec::new(),
            resolved: None,
            at_ms: 3,
        };
        let error = ChatItem::Error {
            message: "Failure".into(),
            at_ms: 4,
        };
        let items = vec![
            user,
            message("Working…"),
            tool(),
            thought(),
            ChatItem::Child {
                id: "child".into(),
                parent_id: "root".into(),
                title: None,
                items: vec![message("child result")],
                at_ms: 2,
            },
            message("# Final"),
            thought(),
            message(" **answer**"),
            permission,
            error,
        ];
        let visible = project(&items, true, false, true);
        assert_eq!(visible.len(), 4);
        assert!(visible.iter().any(
            |item| matches!(item,ChatItem::AgentText {text,..} if text=="# Final **answer**")
        ));
        assert!(!visible.iter().any(|item| matches!(
            item,
            ChatItem::Tool { .. } | ChatItem::Child { .. } | ChatItem::Thought { .. }
        )));
        let streaming = project(&items, true, true, false);
        assert!(
            !streaming
                .iter()
                .any(|item| matches!(item, ChatItem::AgentText { .. }))
        );
        assert!(
            streaming
                .iter()
                .any(|item| matches!(item, ChatItem::Permission { resolved: None, .. }))
        );
        assert!(
            streaming
                .iter()
                .any(|item| matches!(item, ChatItem::Error { .. }))
        );
        assert_eq!(items.len(), 10); // no history was removed or changed
    }

    #[test]
    fn saved_legacy_chunks_are_coalesced_without_crossing_tools_or_changing_indices() {
        let items = vec![
            message("# Wea"),
            thought(),
            message("ther\n\n**Fine**"),
            tool(),
            message("New step"),
        ];
        let visible = project(&items, false, false, true);
        assert_eq!(visible.len(), items.len());
        assert!(
            matches!(&visible[0],ChatItem::AgentText {text,..} if text=="# Weather\n\n**Fine**")
        );
        assert!(matches!(&visible[2],ChatItem::AgentText {text,..} if text.is_empty()));
        assert!(matches!(&visible[3], ChatItem::Tool { .. }));
    }

    #[test]
    fn chat_windows_grow_from_user_turns_and_never_stall_mid_turn() {
        let user = |text: &str| ChatItem::User {
            text: text.into(),
            timing: None,
            at_ms: 0,
        };
        let mut items = vec![user("start")];
        items.extend((0..600).map(|_| message("work")));
        items.push(user("middle"));
        items.extend((0..50).map(|_| message("tail")));

        // 652 items, window 120: cutoff 532 lands inside the long first turn,
        // so it starts mid-turn instead of jumping back to item 1.
        assert_eq!(window_start(&items, 120), 532);

        // A nearby user turn is a nicer boundary than a torn agent group.
        let mut short = Vec::new();
        for turn in 0..6 {
            short.push(user(&format!("turn {turn}")));
            short.extend((0..19).map(|_| message("work")));
        }
        assert_eq!(short.len(), 120);
        assert_eq!(window_start(&short, 40), 80);
        assert_eq!(window_start(&short, 500), 0);
    }

    #[test]
    fn legacy_tool_text_becomes_a_tool_card_instead_of_one_giant_markdown_block() {
        let mut items = vec![
            message("before"),
            message(
                "[Tool · shell] ls -la\nStatus: completed\nInput: {\"cmd\":\"ls\"}\nOutput: total",
            ),
            thought(),
            message("after"),
        ];
        normalize_legacy(&mut items);
        assert!(matches!(
            &items[1],
            ChatItem::Tool { view, .. }
                if view.title == "shell" && view.detail.contains("ls -la") && view.detail.contains("total")
        ));
        let visible = project(&items, false, false, true);
        assert_eq!(
            visible
                .iter()
                .filter(|item| matches!(item, ChatItem::Tool { .. }))
                .count(),
            1
        );
        assert!(matches!(&visible[0], ChatItem::AgentText { text, .. } if text == "before"));
    }

    #[test]
    fn final_only_does_not_hide_answers_from_completed_previous_turns() {
        let items = vec![
            ChatItem::User {
                text: "One".into(),
                timing: None,
                at_ms: 1,
            },
            message("Answer one"),
            ChatItem::User {
                text: "Two".into(),
                timing: None,
                at_ms: 2,
            },
            message("Still typing"),
        ];
        let visible = project(&items, true, true, false);
        assert_eq!(
            visible
                .iter()
                .filter(|item| matches!(item, ChatItem::AgentText { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn final_only_can_stream_partial_markdown_before_completion_without_revealing_tools() {
        let mut items = vec![
            ChatItem::User {
                text: "Question".into(),
                timing: None,
                at_ms: 1,
            },
            message("Working…"),
            tool(),
            message("# Par"),
        ];
        let visible = project(&items, true, true, true);
        assert!(
            matches!(&visible[1], ChatItem::AgentText { text, streaming: true, .. } if text == "# Par")
        );
        if let ChatItem::AgentText { text, .. } = items.last_mut().unwrap() {
            text.push_str("tial **answer**");
        }
        let visible = project(&items, true, true, true);
        assert!(
            matches!(&visible[1], ChatItem::AgentText { text, .. } if text == "# Partial **answer**")
        );
        assert!(
            !visible
                .iter()
                .any(|item| matches!(item, ChatItem::Tool { .. }))
        );
        assert!(
            !project(&items, true, true, false)
                .iter()
                .any(|item| matches!(item, ChatItem::AgentText { .. }))
        );
        assert!(matches!(
            &project(&items, true, false, false)[1],
            ChatItem::AgentText {
                streaming: false,
                ..
            }
        ));
    }
}
