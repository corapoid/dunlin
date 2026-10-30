//! Follow streaming text only while the reader is at the live edge.
use dunlin_ui::{ScrollHandle, px};

pub(crate) struct ChatScroll {
    pub handle: ScrollHandle,
    session: String,
    jump_pending: bool,
    following: bool,
}

impl Default for ChatScroll {
    fn default() -> Self {
        Self {
            handle: ScrollHandle::new(),
            session: String::new(),
            jump_pending: true,
            following: true,
        }
    }
}

fn near_bottom(offset: f32, maximum: f32) -> bool {
    offset.is_finite() && maximum.is_finite() && (maximum + offset).max(0.) <= 24.
}

impl ChatScroll {
    pub(crate) fn jump_to_latest(&mut self) {
        self.jump_pending = true;
        self.following = true;
    }

    pub(crate) fn user_scrolled(&mut self) {
        self.following = near_bottom(
            self.handle.offset().y / px(1.),
            self.handle.max_offset().y / px(1.),
        );
    }

    pub(crate) fn pause_following(&mut self) {
        self.following = false;
        self.jump_pending = false;
    }

    /// Called before layout. GPUI's
    /// scroll_to_bottom is applied *after* measuring the new streaming content.
    pub(crate) fn prepare(&mut self, session: &str) -> bool {
        if self.session != session {
            self.session = session.into();
            self.handle = ScrollHandle::new();
            self.jump_pending = true;
            self.following = true;
        }
        // Content growth can move the old offset away from the new bottom.
        // Only an actual user scroll is allowed to pause following, not layout.
        let follow = std::mem::take(&mut self.jump_pending) || self.following;
        if follow {
            self.handle.scroll_to_bottom();
        }
        follow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_edge_follows_but_reading_history_does_not() {
        assert!(near_bottom(0., 0.));
        assert!(near_bottom(-1000., 1000.));
        assert!(near_bottom(-980., 1000.));
        assert!(!near_bottom(-900., 1000.));
        assert!(!near_bottom(0., 1000.));
        assert!(!near_bottom(f32::NAN, 1000.));
    }

    #[test]
    fn switching_conversations_and_sending_resume_following() {
        let mut scroll = ChatScroll::default();
        assert!(scroll.prepare("first"));
        scroll.jump_to_latest();
        assert!(scroll.prepare("first"));
        assert!(!scroll.jump_pending);
        assert!(scroll.prepare("second"));
        assert_eq!(scroll.session, "second");
    }

    #[test]
    fn content_growth_does_not_turn_off_following_without_a_user_scroll() {
        let mut scroll = ChatScroll::default();
        assert!(scroll.prepare("streaming"));
        for _ in 0..100 {
            assert!(scroll.prepare("streaming"));
        }
        scroll.pause_following();
        assert!(!scroll.prepare("streaming"));
        scroll.jump_to_latest();
        assert!(scroll.prepare("streaming"));
    }
}
