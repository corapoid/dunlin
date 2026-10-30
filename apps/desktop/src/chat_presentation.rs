//! Presentation filters never alter the stored transcript or permission flow.
use crate::harness::ChatItem;

pub(crate) fn project(
    items: &[ChatItem],
    final_only: bool,
    running: bool,
    stream: bool,
) -> Vec<ChatItem> {
    if !final_only {
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
        return visible;
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
    output
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
