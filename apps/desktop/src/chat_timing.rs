//! Optional display metadata; unknown historical times are never fabricated.
use crate::harness::{ChatItem, TurnTiming};
use chrono::{DateTime, Local, Utc};

pub(crate) fn local_time(at_ms: u64) -> Option<String> {
    if at_ms == 0 {
        return None;
    }
    let date = DateTime::<Utc>::from_timestamp_millis(i64::try_from(at_ms).ok()?)?;
    Some(date.with_timezone(&Local).format("%H:%M").to_string())
}

pub(crate) fn agent_caption(
    items: &[ChatItem],
    timing: Option<TurnTiming>,
    times: bool,
    duration: bool,
) -> Option<String> {
    let first_response = items.iter().find_map(|item| {
        if let ChatItem::AgentText { at_ms, text, .. } = item {
            (!text.is_empty() && *at_ms > 0).then_some(*at_ms)
        } else {
            None
        }
    });
    // A queued user message may already be visible while the preceding turn
    // streams. Never attribute that response to a later dispatched prompt.
    let timing = timing.filter(|timing| {
        first_response.is_none_or(|at| {
            at >= timing.started_at_ms && timing.finished_at_ms.is_none_or(|end| at <= end)
        })
    });
    let at = timing.and_then(|timing| timing.finished_at_ms).or_else(|| {
        items.iter().rev().find_map(|item| {
            if let ChatItem::AgentText { at_ms, text, .. } = item {
                (!text.is_empty()).then_some(*at_ms)
            } else {
                None
            }
        })
    });
    let mut parts = Vec::new();
    if times && let Some(time) = at.and_then(local_time) {
        parts.push(time);
    }
    if duration && let Some(ms) = timing.and_then(|timing| timing.elapsed_ms) {
        parts.push(format!("({})", crate::app::format_ms(ms)));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_times_and_duration_do_not_invent_missing_legacy_metadata() {
        assert!(local_time(0).is_none());
        assert!(local_time(u64::MAX).is_none());
        assert_eq!(local_time(1_700_000_000_000).unwrap().len(), 5);
        assert!(agent_caption(&[], None, true, true).is_none());
        let timing = Some(TurnTiming {
            started_at_ms: 1_700_000_000_000,
            finished_at_ms: Some(1_700_000_001_500),
            elapsed_ms: Some(1500),
            usage: None,
            usage_before: None,
        });
        assert_eq!(
            agent_caption(&[], timing, false, true).unwrap(),
            format!("({})", crate::app::format_ms(1500))
        );
        assert!(agent_caption(&[], timing, false, false).is_none());
        assert!(
            agent_caption(&[], timing, true, true)
                .unwrap()
                .contains('(')
        );
        let earlier = [ChatItem::AgentText {
            text: "previous turn".into(),
            streaming: false,
            message_id: None,
            at_ms: 1_699_999_999_000,
        }];
        assert!(agent_caption(&earlier, timing, false, true).is_none());
    }
}
