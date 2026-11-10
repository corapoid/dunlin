//! Persisted conversation items. Preserve serde names and defaults so existing
//! desktop transcripts need no data migration when shared with headless clients.
use dunlin_agent::{HarnessUpdate, PermissionChoice, PlanItem, ToolCallView};
use dunlin_domain::ToolStatus;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TurnTiming {
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub elapsed_ms: Option<u64>,
    #[serde(default)]
    pub usage: Option<dunlin_agent::TokenUsage>,
    #[serde(default)]
    pub usage_before: Option<dunlin_agent::TokenUsage>,
    #[serde(default)]
    pub speaker: Option<Speaker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Speaker {
    pub harness: String,
    pub model: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ChatItem {
    User {
        text: String,
        #[serde(default)]
        timing: Option<TurnTiming>,
        #[serde(default)]
        at_ms: u64,
    },
    AgentText {
        text: String,
        streaming: bool,
        #[serde(default)]
        message_id: Option<String>,
        #[serde(default)]
        at_ms: u64,
    },
    Thought {
        text: String,
        streaming: bool,
        #[serde(default)]
        message_id: Option<String>,
        #[serde(default)]
        at_ms: u64,
    },
    Tool {
        view: ToolCallView,
        duration_ms: u64,
        #[serde(default)]
        at_ms: u64,
    },
    Plan {
        items: Vec<PlanItem>,
        #[serde(default)]
        at_ms: u64,
    },
    Permission {
        request_id: String,
        title: String,
        options: Vec<PermissionChoice>,
        resolved: Option<String>,
        #[serde(default)]
        at_ms: u64,
    },
    Error {
        message: String,
        #[serde(default)]
        at_ms: u64,
    },
    Child {
        id: String,
        parent_id: String,
        title: Option<String>,
        items: Vec<ChatItem>,
        #[serde(default)]
        at_ms: u64,
    },
}

#[derive(Clone, Copy)]
pub enum StreamKind {
    Message,
    Thought,
}

pub fn append_stream(transcript: &mut Vec<ChatItem>, kind: StreamKind, text: String, at_ms: u64) {
    append_identified(transcript, kind, text, at_ms, None);
}

pub fn append_identified(
    transcript: &mut Vec<ChatItem>,
    kind: StreamKind,
    text: String,
    at_ms: u64,
    id: Option<String>,
) {
    if let Some(id) = &id {
        for item in transcript.iter_mut().rev() {
            if matches!(item, ChatItem::User { .. }) {
                break;
            }
            match (kind, item) {
                (
                    StreamKind::Message,
                    ChatItem::AgentText {
                        text: body,
                        message_id: Some(existing),
                        ..
                    },
                )
                | (
                    StreamKind::Thought,
                    ChatItem::Thought {
                        text: body,
                        message_id: Some(existing),
                        ..
                    },
                ) if existing == id => {
                    body.push_str(&text);
                    return;
                }
                _ => {}
            }
        }
    }
    match (kind, transcript.last_mut()) {
        (
            StreamKind::Message,
            Some(ChatItem::AgentText {
                text: body,
                streaming,
                message_id,
                ..
            }),
        )
        | (
            StreamKind::Thought,
            Some(ChatItem::Thought {
                text: body,
                streaming,
                message_id,
                ..
            }),
        ) if *streaming && *message_id == id => {
            body.push_str(&text);
            return;
        }
        _ => {}
    }
    transcript.push(match kind {
        StreamKind::Message => ChatItem::AgentText {
            text,
            streaming: true,
            message_id: id,
            at_ms,
        },
        StreamKind::Thought => ChatItem::Thought {
            text,
            streaming: true,
            message_id: id,
            at_ms,
        },
    });
}

pub fn append_child(
    transcript: &mut Vec<ChatItem>,
    id: String,
    parent_id: String,
    title: Option<String>,
    update: HarnessUpdate,
    at_ms: u64,
) {
    if !transcript
        .iter()
        .any(|item| matches!(item, ChatItem::Child { id: existing, .. } if existing == &id))
    {
        transcript.push(ChatItem::Child {
            id: id.clone(),
            parent_id,
            title: title.clone(),
            items: Vec::new(),
            at_ms,
        });
    }
    let Some(ChatItem::Child {
        items,
        title: current,
        ..
    }) = transcript
        .iter_mut()
        .find(|item| matches!(item, ChatItem::Child { id: existing, .. } if existing == &id))
    else {
        return;
    };
    if title.is_some() {
        *current = title;
    }
    match update {
        HarnessUpdate::AgentMessage(text) => append_stream(items, StreamKind::Message, text, at_ms),
        HarnessUpdate::AgentThought(text) => append_stream(items, StreamKind::Thought, text, at_ms),
        HarnessUpdate::MessageChunk { id, text, thought } => append_identified(
            items,
            if thought {
                StreamKind::Thought
            } else {
                StreamKind::Message
            },
            text,
            at_ms,
            Some(id),
        ),
        HarnessUpdate::ToolCall(view) => items.push(ChatItem::Tool {
            view,
            duration_ms: 0,
            at_ms,
        }),
        HarnessUpdate::ToolCallUpdate(delta) => {
            if let Some(ChatItem::Tool {
                view,
                duration_ms,
                at_ms: started,
                ..
            }) = items
                .iter_mut()
                .find(|item| matches!(item, ChatItem::Tool { view, .. } if view.id == delta.id))
            {
                if delta
                    .status
                    .is_some_and(|status| matches!(status, ToolStatus::Done | ToolStatus::Failed))
                    && !matches!(view.status, ToolStatus::Done | ToolStatus::Failed)
                {
                    *duration_ms = at_ms.saturating_sub(*started);
                }
                view.apply(delta);
            }
        }
        HarnessUpdate::Plan(plan) => items.push(ChatItem::Plan { items: plan, at_ms }),
        HarnessUpdate::Child {
            id,
            parent_id,
            title,
            update,
        } => append_child(items, id, parent_id, title, *update, at_ms),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{from_value, json, to_value};

    #[test]
    fn legacy_items_without_timing_or_message_ids_still_load() {
        let items: Vec<ChatItem> = from_value(json!([
            {"User":{"text":"hello"}},
            {"AgentText":{"text":"hi", "streaming":false}},
            {"Child":{"id":"c", "parent_id":"p", "title":null, "items":[]}}
        ]))
        .unwrap();
        assert!(matches!(
            &items[0],
            ChatItem::User {
                timing: None,
                at_ms: 0,
                ..
            }
        ));
        assert!(matches!(
            &items[1],
            ChatItem::AgentText {
                message_id: None,
                at_ms: 0,
                ..
            }
        ));
        let roundtrip: Vec<ChatItem> = from_value(to_value(&items).unwrap()).unwrap();
        assert_eq!(to_value(roundtrip).unwrap(), to_value(items).unwrap());
    }

    #[test]
    fn timing_serialization_keeps_optional_failure_omitted() {
        let timing: TurnTiming =
            from_value(json!({"started_at_ms":42, "finished_at_ms":null, "elapsed_ms":null}))
                .unwrap();
        assert!(to_value(timing).unwrap().get("failure").is_none());
    }

    #[test]
    fn identified_chunks_merge_across_thoughts_but_not_across_user_turns() {
        let mut items = Vec::new();
        append_identified(
            &mut items,
            StreamKind::Message,
            "one".into(),
            1,
            Some("m".into()),
        );
        append_identified(
            &mut items,
            StreamKind::Thought,
            "reason".into(),
            2,
            Some("t".into()),
        );
        append_identified(
            &mut items,
            StreamKind::Message,
            "two".into(),
            3,
            Some("m".into()),
        );
        assert_eq!(items.len(), 2);
        assert!(
            matches!(&items[0], ChatItem::AgentText { text, at_ms: 1, .. } if text == "onetwo")
        );
        items.push(ChatItem::User {
            text: "next".into(),
            timing: None,
            at_ms: 4,
        });
        append_identified(
            &mut items,
            StreamKind::Message,
            "new".into(),
            5,
            Some("m".into()),
        );
        assert_eq!(items.len(), 4);
        assert!(matches!(&items[3], ChatItem::AgentText { text, .. } if text == "new"));
    }

    #[test]
    fn child_updates_reuse_the_child_and_update_its_title() {
        let mut items = Vec::new();
        append_child(
            &mut items,
            "child".into(),
            "parent".into(),
            None,
            HarnessUpdate::AgentMessage("one".into()),
            1,
        );
        append_child(
            &mut items,
            "child".into(),
            "parent".into(),
            Some("Worker".into()),
            HarnessUpdate::AgentMessage("two".into()),
            2,
        );
        assert_eq!(items.len(), 1);
        let ChatItem::Child { title, items, .. } = &items[0] else {
            panic!("child item expected");
        };
        assert_eq!(title.as_deref(), Some("Worker"));
        assert_eq!(items.len(), 1);
        assert!(matches!(&items[0], ChatItem::AgentText { text, .. } if text == "onetwo"));
    }
}
