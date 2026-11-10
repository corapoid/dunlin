//! Per-session pending prompts. No scheduler, persistence, budget bypass or UI:
//! callers still authorize a request immediately before taking it for delivery.
use std::collections::VecDeque;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedPrompt {
    pub id: u64,
    pub text: String,
    pub created_at_ms: u64,
    pub skill_context: String,
    pub skill_ids: Vec<String>,
}

#[derive(Default)]
pub struct PromptQueue {
    pending: VecDeque<QueuedPrompt>,
    next_id: u64,
}

impl PromptQueue {
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Return owned snapshots so UI drafts cannot change pending wire content.
    pub fn snapshot(&self) -> Vec<QueuedPrompt> {
        self.pending.iter().cloned().collect()
    }

    pub fn enqueue(&mut self, text: String, created_at_ms: u64) -> QueuedPrompt {
        loop {
            self.next_id = self.next_id.wrapping_add(1).max(1);
            if !self.pending.iter().any(|prompt| prompt.id == self.next_id) {
                break;
            }
        }
        let prompt = QueuedPrompt {
            id: self.next_id,
            text,
            created_at_ms,
            skill_context: String::new(),
            skill_ids: Vec::new(),
        };
        self.pending.push_back(prompt.clone());
        prompt
    }

    pub fn set_skill_context(&mut self, id: u64, context: String, skill_ids: Vec<String>) -> bool {
        let Some(prompt) = self.pending.iter_mut().find(|prompt| prompt.id == id) else {
            return false;
        };
        prompt.skill_context = context;
        prompt.skill_ids = skill_ids;
        true
    }

    pub fn remove(&mut self, id: u64) -> Option<QueuedPrompt> {
        let index = self.pending.iter().position(|prompt| prompt.id == id)?;
        self.pending.remove(index)
    }

    pub fn move_by(&mut self, id: u64, delta: isize) -> bool {
        let Some(index) = self.pending.iter().position(|prompt| prompt.id == id) else {
            return false;
        };
        let target = index
            .saturating_add_signed(delta)
            .min(self.pending.len() - 1);
        if target == index {
            return false;
        }
        let prompt = self.pending.remove(index).expect("index found in queue");
        self.pending.insert(target, prompt);
        true
    }

    pub fn pop_front(&mut self) -> Option<QueuedPrompt> {
        self.pending.pop_front()
    }

    /// Return a taken prompt after pre-send preparation failed. Preserve its ID,
    /// timestamp and reviewed context; do not enqueue it as a new request.
    pub fn restore_front(&mut self, prompt: QueuedPrompt) {
        self.pending.push_front(prompt);
    }

    /// Clearing does not reset IDs: late UI actions must not target a new item.
    pub fn clear(&mut self) -> usize {
        let count = self.pending.len();
        self.pending.clear();
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_texts_have_distinct_ids_and_explicit_timestamps() {
        let mut queue = PromptQueue::default();
        let first = queue.enqueue("same".into(), 42);
        let second = queue.enqueue("same".into(), 42);
        assert_ne!(first.id, second.id);
        assert_eq!(queue.pop_front(), Some(first));
        assert_eq!(queue.pop_front(), Some(second));
        assert!(queue.is_empty());
    }

    #[test]
    fn editing_and_removing_are_id_scoped_not_text_scoped() {
        let mut queue = PromptQueue::default();
        let first = queue.enqueue("same".into(), 1);
        let second = queue.enqueue("same".into(), 1);
        assert!(queue.set_skill_context(second.id, "reviewed".into(), vec!["skill".into()]));
        assert!(!queue.set_skill_context(99, "wrong".into(), vec![]));
        assert_eq!(queue.remove(first.id), Some(first.clone()));
        assert!(queue.remove(first.id).is_none());
        assert_eq!(queue.snapshot()[0].skill_context, "reviewed");
    }

    #[test]
    fn snapshots_cannot_mutate_the_queue_or_another_prompt() {
        let mut queue = PromptQueue::default();
        let prompt = queue.enqueue("first".into(), 1);
        queue.set_skill_context(prompt.id, "revision one".into(), vec!["skill".into()]);
        let mut snapshot = queue.snapshot();
        snapshot[0].text = "edited draft".into();
        snapshot[0].skill_context = "revision two".into();
        queue.enqueue("second".into(), 2);
        assert_eq!(queue.snapshot()[0].text, "first");
        assert_eq!(queue.snapshot()[0].skill_context, "revision one");
        assert!(queue.snapshot()[1].skill_context.is_empty());
    }

    #[test]
    fn reorder_clamps_extreme_deltas_and_keeps_identity() {
        let mut queue = PromptQueue::default();
        let first = queue.enqueue("a".into(), 1);
        let second = queue.enqueue("b".into(), 2);
        let third = queue.enqueue("c".into(), 3);
        assert!(queue.move_by(second.id, isize::MAX));
        assert_eq!(queue.snapshot(), vec![first.clone(), third, second.clone()]);
        assert!(!queue.move_by(second.id, isize::MAX));
        assert!(queue.move_by(second.id, isize::MIN));
        assert_eq!(queue.pop_front(), Some(second));
        assert!(!queue.move_by(99, -1));
        assert!(!queue.move_by(first.id, 0));
    }

    #[test]
    fn failed_preparation_restores_the_same_request_before_its_successors() {
        let mut queue = PromptQueue::default();
        let first = queue.enqueue("a".into(), 1);
        queue.set_skill_context(first.id, "reviewed".into(), vec!["skill".into()]);
        let pending = queue.pop_front().unwrap();
        queue.enqueue("b".into(), 2);
        queue.restore_front(pending.clone());
        assert_eq!(queue.pop_front(), Some(pending));
        assert_eq!(queue.pop_front().unwrap().text, "b");
    }

    #[test]
    fn clear_retains_id_generation_and_old_ui_actions_do_not_match_new_prompts() {
        let mut queue = PromptQueue::default();
        let old = queue.enqueue("old".into(), 1);
        assert_eq!(queue.clear(), 1);
        assert_eq!(queue.clear(), 0);
        let new = queue.enqueue("new".into(), 2);
        assert_ne!(old.id, new.id);
        assert!(queue.remove(old.id).is_none());
        assert_eq!(queue.pop_front(), Some(new));
    }

    #[test]
    fn id_wraparound_does_not_reuse_a_live_prompt_id() {
        let mut queue = PromptQueue::default();
        let first = queue.enqueue("first".into(), 1);
        queue.next_id = u64::MAX;
        assert_ne!(queue.enqueue("next".into(), 2).id, first.id);
    }
}
