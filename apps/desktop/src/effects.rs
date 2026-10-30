//! Reversible sidebar motion and a shared, input-transparent overlay backdrop.
use std::time::Instant;

mod backdrop;
pub(crate) use backdrop::{
    OverlayBackdrop, for_panel_window, for_window, menu_item, menu_surface, rounded_probe,
    rounded_scroll_probe,
};

pub(crate) fn panel_fill(backdrop: &OverlayBackdrop, color: dunlin_ui::Hsla) -> dunlin_ui::Hsla {
    if backdrop.is_panel_plane() {
        color.opacity(0.14)
    } else {
        color
    }
}
pub(crate) fn panel_border(backdrop: &OverlayBackdrop, p: &dunlin_ui::Palette) -> dunlin_ui::Hsla {
    if backdrop.is_panel_plane() {
        p.fg.opacity(0.14)
    } else {
        p.border_strong
    }
}
pub(crate) fn panel_shadow() -> Vec<dunlin_ui::BoxShadow> {
    use dunlin_ui::{BoxShadow, black, px};
    vec![BoxShadow::new(px(0.), px(8.), black().opacity(0.16)).blur_radius(px(24.))]
}

#[derive(Default)]
pub(crate) struct SidebarMotion {
    value: f32,
    from: f32,
    target: f32,
    started: Option<Instant>,
    initialized: bool,
}
impl SidebarMotion {
    pub fn sample(&mut self, visible: bool) -> (f32, bool) {
        let target = if visible { 1. } else { 0. };
        if !self.initialized {
            self.initialized = true;
            self.value = target;
            self.target = target;
        }
        if self.target != target {
            self.from = self.value;
            self.target = target;
            self.started = Some(Instant::now());
        }
        if let Some(started) = self.started {
            let t = (started.elapsed().as_secs_f32() / 0.19).min(1.);
            self.value = self.from + (self.target - self.from) * (1. - (1. - t).powi(3));
            if t >= 1. {
                self.started = None;
            }
        }
        (self.value, self.started.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::SidebarMotion;
    #[test]
    fn sidebar_starts_at_rest_and_reverses_without_jumping() {
        let mut motion = SidebarMotion::default();
        assert_eq!(motion.sample(true), (1., false));
        let (start, moving) = motion.sample(false);
        assert!(moving);
        assert!((start - 1.).abs() < 0.01);
        let (reverse, moving) = motion.sample(true);
        assert!(moving);
        assert!((reverse - start).abs() < 0.01);
    }
}
