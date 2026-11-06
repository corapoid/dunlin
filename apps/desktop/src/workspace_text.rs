//! Text-only workspace zoom; sidebar, tabs and Settings keep their normal sizes.
use dunlin_ui::*;

struct TextZoom(f32);
impl Global for TextZoom {}

pub(crate) fn scale(cx: &App) -> f32 {
    cx.try_global::<TextZoom>().map_or(1., |zoom| zoom.0)
}

pub(crate) fn size(base: f32, cx: &App) -> Pixels {
    px(base * scale(cx))
}

fn next(current: f32, delta: f32) -> f32 {
    ((current + delta) * 10.).round().clamp(7., 20.) / 10.
}

pub(crate) fn adjust(delta: f32, cx: &mut App) {
    // shortcut: zoom lasts for this run; persist it when a saved zoom preference is requested.
    cx.set_global(TextZoom(next(scale(cx), delta)));
}

pub(crate) fn reset(cx: &mut App) {
    cx.set_global(TextZoom(1.));
}

#[cfg(test)]
mod tests {
    #[test]
    fn zoom_is_bounded_and_reversible_without_floating_point_drift() {
        assert_eq!(super::next(super::next(1., 0.1), -0.1), 1.);
        assert_eq!(super::next(2., 0.1), 2.);
        assert_eq!(super::next(0.7, -0.1), 0.7);
    }
}
