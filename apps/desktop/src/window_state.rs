//! Remember the macOS window frame between launches.

use std::path::PathBuf;

use dunlin_ui::*;
use serde::{Deserialize, Serialize};

const MIN_WIDTH: f32 = 1100.0;
const MIN_HEIGHT: f32 = 720.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WindowGeometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A display's usable area in the same coordinate space as the window frame.
/// Keeping this independent of GPUI makes the clamping logic easy to test.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DisplayRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl DisplayRect {
    pub(crate) fn from_bounds(bounds: &Bounds<Pixels>) -> Self {
        Self {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
        }
    }
}

fn path() -> Option<PathBuf> {
    crate::app::data_dir().map(|directory| directory.join("window.json"))
}

pub(crate) fn load() -> Option<WindowGeometry> {
    let path = path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let geometry: WindowGeometry = serde_json::from_str(&text).ok()?;
    (geometry.width >= 900.0 && geometry.height >= 600.0).then_some(geometry)
}

pub(crate) fn save(geometry: WindowGeometry) {
    let Some(path) = path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(text) = serde_json::to_string_pretty(&geometry) else {
        return;
    };
    let temporary = path.with_extension("json.tmp");
    if std::fs::write(&temporary, text).is_ok() {
        let _ = std::fs::rename(temporary, path);
    }
}

pub(crate) fn from_bounds(bounds: WindowBounds) -> WindowGeometry {
    let bounds = match bounds {
        WindowBounds::Windowed(bounds)
        | WindowBounds::Maximized(bounds)
        | WindowBounds::Fullscreen(bounds) => bounds,
    };
    WindowGeometry {
        x: f32::from(bounds.origin.x),
        y: f32::from(bounds.origin.y),
        width: f32::from(bounds.size.width),
        height: f32::from(bounds.size.height),
    }
}

pub(crate) fn to_bounds(geometry: WindowGeometry) -> WindowBounds {
    WindowBounds::Windowed(Bounds::new(
        point(px(geometry.x), px(geometry.y)),
        size(px(geometry.width), px(geometry.height)),
    ))
}

/// Keep a restored frame on a display that is still connected. The window is
/// moved and shrunk only when necessary, so multi-monitor layouts keep their
/// exact position.
pub(crate) fn clamp(mut geometry: WindowGeometry, displays: &[DisplayRect]) -> WindowGeometry {
    let Some(display) = best_display(&geometry, displays) else {
        return geometry;
    };

    geometry.width = geometry
        .width
        .min(display.width)
        .max(MIN_WIDTH.min(display.width));
    geometry.height = geometry
        .height
        .min(display.height)
        .max(MIN_HEIGHT.min(display.height));

    let max_x = display.x + display.width - geometry.width;
    let max_y = display.y + display.height - geometry.height;
    geometry.x = geometry.x.max(display.x).min(max_x);
    geometry.y = geometry.y.max(display.y).min(max_y);
    geometry
}

fn best_display<'a>(
    geometry: &WindowGeometry,
    displays: &'a [DisplayRect],
) -> Option<&'a DisplayRect> {
    if displays.is_empty() {
        return None;
    }

    let window_left = geometry.x;
    let window_top = geometry.y;
    let window_right = geometry.x + geometry.width;
    let window_bottom = geometry.y + geometry.height;

    displays
        .iter()
        .max_by(|a, b| {
            overlap_area(a, window_left, window_top, window_right, window_bottom)
                .total_cmp(&overlap_area(
                    b,
                    window_left,
                    window_top,
                    window_right,
                    window_bottom,
                ))
                .then_with(|| {
                    let a_distance = center_distance(a, geometry);
                    let b_distance = center_distance(b, geometry);
                    b_distance.total_cmp(&a_distance)
                })
        })
        .or_else(|| displays.first())
}

fn overlap_area(display: &DisplayRect, left: f32, top: f32, right: f32, bottom: f32) -> f32 {
    let display_right = display.x + display.width;
    let display_bottom = display.y + display.height;

    let width = right.min(display_right) - left.max(display.x);
    let height = bottom.min(display_bottom) - top.max(display.y);
    if width <= 0.0 || height <= 0.0 {
        0.0
    } else {
        width * height
    }
}

fn center_distance(display: &DisplayRect, geometry: &WindowGeometry) -> f32 {
    let display_x = display.x + display.width / 2.0;
    let display_y = display.y + display.height / 2.0;
    let window_x = geometry.x + geometry.width / 2.0;
    let window_y = geometry.y + geometry.height / 2.0;
    (display_x - window_x).powi(2) + (display_y - window_y).powi(2)
}

#[cfg(test)]
mod tests {
    use super::{DisplayRect, WindowGeometry, clamp, from_bounds, to_bounds};

    fn display(x: f32, y: f32, width: f32, height: f32) -> DisplayRect {
        DisplayRect {
            x,
            y,
            width,
            height,
        }
    }

    fn geometry(x: f32, y: f32, width: f32, height: f32) -> WindowGeometry {
        WindowGeometry {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn geometry_round_trips_through_bounds() {
        let geometry = geometry(120.0, 90.0, 1360.0, 860.0);
        let round_trip = from_bounds(to_bounds(geometry));
        assert_eq!(round_trip.x, geometry.x);
        assert_eq!(round_trip.y, geometry.y);
        assert_eq!(round_trip.width, geometry.width);
        assert_eq!(round_trip.height, geometry.height);
    }

    #[test]
    fn offscreen_window_moves_inside_the_display() {
        let displays = [display(0.0, 0.0, 1440.0, 900.0)];
        let clamped = clamp(geometry(5000.0, -800.0, 1360.0, 860.0), &displays);
        assert_eq!(clamped.x, 80.0);
        assert_eq!(clamped.y, 0.0);
    }

    #[test]
    fn oversized_window_shrinks_to_the_display() {
        let displays = [display(0.0, 0.0, 1280.0, 800.0)];
        let clamped = clamp(geometry(0.0, 0.0, 2400.0, 1600.0), &displays);
        assert_eq!(clamped.width, 1280.0);
        assert_eq!(clamped.height, 800.0);
    }

    #[test]
    fn window_on_a_second_display_keeps_its_position() {
        let displays = [
            display(0.0, 0.0, 1440.0, 900.0),
            display(1440.0, 0.0, 1920.0, 1080.0),
        ];
        let clamped = clamp(geometry(1800.0, 120.0, 1360.0, 860.0), &displays);
        assert_eq!(clamped.x, 1800.0);
        assert_eq!(clamped.y, 120.0);
    }
}
