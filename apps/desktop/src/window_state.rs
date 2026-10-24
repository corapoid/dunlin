//! Remember the macOS window frame between launches.

use std::path::PathBuf;

use kestrel_ui::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WindowGeometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
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

#[cfg(test)]
mod tests {
    use super::{WindowGeometry, from_bounds, to_bounds};

    #[test]
    fn geometry_round_trips_through_bounds() {
        let geometry = WindowGeometry {
            x: 120.0,
            y: 90.0,
            width: 1360.0,
            height: 860.0,
        };
        let round_trip = from_bounds(to_bounds(geometry));
        assert_eq!(round_trip.x, geometry.x);
        assert_eq!(round_trip.y, geometry.y);
        assert_eq!(round_trip.width, geometry.width);
        assert_eq!(round_trip.height, geometry.height);
    }
}
