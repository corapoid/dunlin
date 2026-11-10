//! Fold inline agent activity without hiding responses or approval gates.
use super::*;

pub(super) fn is_work(item: &ChatItem) -> bool {
    matches!(
        item,
        ChatItem::Thought { .. }
            | ChatItem::Tool { .. }
            | ChatItem::Plan { .. }
            | ChatItem::Child { .. }
    )
}

pub(super) fn card(
    items: &[ChatItem],
    base: usize,
    options: &AgentGroupOptions<'_>,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> (Div, bool) {
    // Local to this session and section; folding one block must not fold all
    // reasoning cards or change the saved reasoning preference.
    let key = format!("agent-work:{}:{base}", options.session);
    let collapsed = window.use_keyed_state(key.clone(), cx, |_, _| false);
    let selected = options
        .expanded
        .is_some_and(|index| (base..base + items.len()).contains(&index));
    // Opening a tool from Timeline must also reveal its enclosing section.
    let open = !*collapsed.read(cx) || selected;
    let steps = items.len();
    let card = card_surface(*p)
        .overflow_hidden()
        .child(crate::gui_probe::target(
            format!("agent-work-toggle-{base}"),
            card_action_row(key, "Toggle agent work", open)
                .toggled(open)
                .icon(IconName::Brain)
                .label("Agent work")
                .on_click(cx.listener(move |this, _, _, cx| {
                    collapsed.update(cx, |collapsed, _| *collapsed = open);
                    if open && selected {
                        this.expanded_tool = None;
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .text_size(px(11.))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(format!(
                            "{steps} {}",
                            if steps == 1 { "step" } else { "steps" }
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
        ));
    (card, open)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_agent_activity_is_foldable() {
        let thought = ChatItem::Thought {
            text: "Inspect the files".into(),
            streaming: false,
            message_id: None,
            at_ms: 0,
        };
        let plan = ChatItem::Plan {
            items: Vec::new(),
            at_ms: 0,
        };
        let response = ChatItem::AgentText {
            text: "Answer".into(),
            streaming: false,
            message_id: None,
            at_ms: 0,
        };
        let permission = ChatItem::Permission {
            request_id: "approval".into(),
            title: "Allow command?".into(),
            options: Vec::new(),
            resolved: None,
            at_ms: 0,
        };
        let error = ChatItem::Error {
            message: "Failed".into(),
            at_ms: 0,
        };
        assert!(is_work(&thought));
        assert!(is_work(&plan));
        assert!(!is_work(&response));
        assert!(!is_work(&permission));
        assert!(!is_work(&error));

        let items = [
            thought.clone(),
            plan.clone(),
            permission,
            thought,
            error,
            plan,
            response,
        ];
        let sections: Vec<_> = items
            .chunk_by(|a, b| is_work(a) == is_work(b))
            .map(|section| (is_work(&section[0]), section.len()))
            .collect();
        assert_eq!(
            sections,
            [
                (true, 2),
                (false, 1),
                (true, 1),
                (false, 1),
                (true, 1),
                (false, 1)
            ]
        );
    }
}
