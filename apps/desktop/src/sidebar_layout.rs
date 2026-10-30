use crate::app::DunlinApp;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

pub(crate) const DEFAULT_WIDTH: f32 = 278.;
pub(crate) fn width(saved: f32, viewport: f32) -> f32 {
    let saved = if saved.is_finite() {
        saved
    } else {
        DEFAULT_WIDTH
    };
    saved.clamp(220., (viewport - 470.).clamp(220., 560.))
}
impl DunlinApp {
    pub(crate) fn finish_sidebar_resize(&mut self, cx: &mut Context<Self>) {
        if self.sidebar_resize.take().is_some() {
            self.persist_settings(cx);
            cx.notify();
        }
    }
}
pub(crate) fn handle(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    div()
        .id("sidebar-resize-handle")
        .aria_label("Drag to resize sidebar; double-click to reset")
        .absolute()
        .right_0()
        .top_0()
        .h_full()
        .w(px(5.))
        .cursor(CursorStyle::ResizeLeftRight)
        .when(this.sidebar_resize.is_some(), |bar| bar.bg(p.border_strong))
        .hover(move |style| style.bg(p.border_strong))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if event.click_count == 2 {
                    this.sidebar_width = DEFAULT_WIDTH;
                    this.sidebar_resize = None;
                    this.persist_settings(cx);
                } else {
                    this.sidebar_resize = Some((
                        f32::from(event.position.x),
                        width(this.sidebar_width, f32::from(window.viewport_size().width)),
                    ));
                }
                cx.notify();
            }),
        )
        .into_any_element()
}
#[cfg(test)]
mod tests {
    #[test]
    fn widths_are_finite_and_leave_space_for_the_main_panel() {
        assert_eq!(super::width(f32::NAN, 1360.), 278.);
        assert_eq!(super::width(100., 1360.), 220.);
        assert_eq!(super::width(900., 1360.), 560.);
        assert_eq!(super::width(500., 800.), 330.);
    }
}
