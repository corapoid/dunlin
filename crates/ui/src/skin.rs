//! Surface material is independent of the system/light/dark appearance.
use crate::*;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Skin {
    Solid,
    Grain,
    #[default]
    #[serde(other)]
    Default,
}

impl Skin {
    pub const ALL: [Self; 3] = [Self::Default, Self::Solid, Self::Grain];

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Solid => "Solid",
            Self::Grain => "Grain",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Default => "Original translucent appearance",
            Self::Solid => "Opaque surfaces, no glass",
            Self::Grain => "Gray light, midnight dark, fine grain",
        }
    }

    pub fn window_background(self, reduce_transparency: bool) -> WindowBackgroundAppearance {
        if cfg!(target_os = "macos") && self != Self::Solid && !reduce_transparency {
            WindowBackgroundAppearance::Blurred
        } else {
            WindowBackgroundAppearance::Opaque
        }
    }
}

const TILE: u32 = 256;

// Balanced monochrome noise: no tint, animation, I/O or random work per frame.
// RenderImage consumes BGRA; monochrome channels are identical in either order.
fn noise_bytes() -> Vec<u8> {
    let mut state = 0x91e1_0da5u32;
    let mut bytes = Vec::with_capacity((TILE * TILE * 4) as usize);
    for _ in 0..TILE * TILE {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let ink = if state & 1 == 0 { 0 } else { 255 };
        let alpha = ((state >> 8) % 10) as u8;
        bytes.extend_from_slice(&[ink, ink, ink, alpha]);
    }
    bytes
}

fn texture() -> Arc<RenderImage> {
    static TEXTURE: OnceLock<Arc<RenderImage>> = OnceLock::new();
    TEXTURE
        .get_or_init(|| {
            let buffer = image::RgbaImage::from_raw(TILE, TILE, noise_bytes()).unwrap();
            Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
        })
        .clone()
}

/// First child of a surface, behind controls/text. Canvas has no hitbox.
/// Rounded clipping is applied to the entire surface, not individual tiles.
pub fn grain_layer(p: Palette, radius: f32) -> Option<AnyElement> {
    if p.skin != Skin::Grain {
        return None;
    }
    let texture = texture();
    Some(
        canvas(
            |bounds, window, _| bounds.intersect(&window.content_mask().bounds),
            move |bounds, _, window, _| {
                // One noise sample per backing pixel, independent of window size.
                let scale = window.scale_factor();
                let side = px(TILE as f32 / scale);
                // Center the grid's overflow: edge tiles stay at least half a
                // tile wide, so rounded corners never collapse on a tiny tail.
                let columns = (bounds.size.width / side).ceil().max(1.);
                let rows = (bounds.size.height / side).ceil().max(1.);
                let origin = point(
                    bounds.left() - (side * columns - bounds.size.width) / 2.,
                    bounds.top() - (side * rows - bounds.size.height) / 2.,
                );
                let mut y = origin.y;
                while y < bounds.bottom() {
                    let mut x = origin.x;
                    while x < bounds.right() {
                        let tile = Bounds::new(point(x, y), size(side, side));
                        let left = x <= bounds.left();
                        let top = y <= bounds.top();
                        let right = x + side >= bounds.right();
                        let bottom = y + side >= bounds.bottom();
                        let corner = |edge| if edge { px(radius) } else { px(0.) };
                        let _ = window.paint_image(
                            bounds,
                            tile,
                            Corners {
                                top_left: corner(top && left),
                                top_right: corner(top && right),
                                bottom_left: corner(bottom && left),
                                bottom_right: corner(bottom && right),
                            },
                            texture.clone(),
                            0,
                            false,
                        );
                        x += side;
                    }
                    y += side;
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full()
        .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_is_static_fine_balanced_and_cached() {
        let bytes = noise_bytes();
        assert_eq!(bytes, noise_bytes());
        assert_eq!(bytes.len(), (TILE * TILE * 4) as usize);
        let mut light = 0;
        for pixel in bytes.as_chunks::<4>().0 {
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(pixel[1], pixel[2]);
            assert!(pixel[3] < 10);
            light += usize::from(pixel[0] == 255);
        }
        assert!((31_000..35_000).contains(&light));
        assert!(Arc::ptr_eq(&texture(), &texture()));
    }

    #[test]
    fn solid_and_accessibility_always_use_an_opaque_window() {
        for skin in Skin::ALL {
            assert_eq!(
                skin.window_background(true),
                WindowBackgroundAppearance::Opaque
            );
        }
        assert_eq!(
            Skin::Solid.window_background(false),
            WindowBackgroundAppearance::Opaque
        );
    }
}
