//! Native regression support. Inspect the rendered scene, not just style values.
use crate::*;

fn inside(
    bounds: Bounds<ScaledPixels>,
    radii: Corners<ScaledPixels>,
    sample: Point<ScaledPixels>,
) -> bool {
    if !bounds.contains(&sample) {
        return false;
    }
    ![
        (bounds.left(), bounds.top(), radii.top_left, 1., 1.),
        (bounds.right(), bounds.top(), radii.top_right, -1., 1.),
        (bounds.left(), bounds.bottom(), radii.bottom_left, 1., -1.),
        (
            bounds.right(),
            bounds.bottom(),
            radii.bottom_right,
            -1.,
            -1.,
        ),
    ]
    .into_iter()
    .any(|(x, y, radius, dx, dy)| {
        let r = radius.0;
        let x = (sample.x - x).0 * dx;
        let y = (sample.y - y).0 * dy;
        x < r && y < r && (x - r).powi(2) + (y - r).powi(2) > r * r
    })
}

fn paints_at(quad: &Quad, sample: Point<ScaledPixels>) -> bool {
    if !quad.content_mask.bounds.contains(&sample)
        || !inside(quad.bounds, quad.corner_radii, sample)
    {
        return false;
    }
    if !quad.background.is_transparent() {
        return true;
    }
    if quad.border_color.is_transparent() || quad.border_style != BorderStyle::default() {
        return false;
    }
    let b = quad.bounds;
    let w = quad.border_widths;
    let inner = Bounds::from_corners(
        point(b.left() + w.left, b.top() + w.top),
        point(b.right() - w.right, b.bottom() - w.bottom),
    );
    let r = quad.corner_radii;
    let inner_radii = Corners {
        top_left: (r.top_left - w.left.max(w.top)).max(ScaledPixels(0.)),
        top_right: (r.top_right - w.right.max(w.top)).max(ScaledPixels(0.)),
        bottom_left: (r.bottom_left - w.left.max(w.bottom)).max(ScaledPixels(0.)),
        bottom_right: (r.bottom_right - w.right.max(w.bottom)).max(ScaledPixels(0.)),
    };
    !inside(inner, inner_radii, sample)
}

fn contains_bounds(outer: Bounds<ScaledPixels>, inner: Bounds<ScaledPixels>) -> bool {
    inner.left() >= outer.left()
        && inner.right() <= outer.right()
        && inner.top() >= outer.top()
        && inner.bottom() <= outer.bottom()
}

/// Check a grid across every curved corner, not just its extreme pixel.
/// Ancestors are ignored; any later opaque child, including hover paint and
/// scroll content, must remain inside the surface's actual painted curve.
pub fn check_painted_corners(
    window: &Window,
    bounds: Bounds<Pixels>,
    rounded_left: bool,
) -> Result<(), String> {
    let target = bounds.scale(window.scale_factor());
    let quads = window.painted_quads();
    let surface = quads
        .iter()
        .find(|quad| {
            // GPUI snaps all four bounds to device pixels (half toward zero).
            // Keep fractional layouts testable without guessing the platform scale.
            (quad.bounds.left() - target.left()).0.abs() <= 0.501
                && (quad.bounds.top() - target.top()).0.abs() <= 0.501
                && (quad.bounds.right() - target.right()).0.abs() <= 0.501
                && (quad.bounds.bottom() - target.bottom()).0.abs() <= 0.501
                && quad.corner_radii.top_right.0 > 0.
                && !quad.background.is_transparent()
        })
        .ok_or_else(|| format!("missing rounded painted surface at {target:?}"))?;
    let target = surface.bounds;
    if (surface.corner_radii.top_left.0 > 0.) != rounded_left {
        return Err("selected first-tab join has the wrong top-left corner".into());
    }
    for (left, top, radius) in [
        (true, true, surface.corner_radii.top_left),
        (false, true, surface.corner_radii.top_right),
        (true, false, surface.corner_radii.bottom_left),
        (false, false, surface.corner_radii.bottom_right),
    ] {
        if radius.0 <= 0. {
            continue;
        }
        for ix in 0..8 {
            for iy in 0..8 {
                let dx = radius * ((ix as f32 + 0.5) / 8.);
                let dy = radius * ((iy as f32 + 0.5) / 8.);
                let sample = point(
                    if left {
                        target.left() + dx
                    } else {
                        target.right() - dx
                    },
                    if top {
                        target.top() + dy
                    } else {
                        target.bottom() - dy
                    },
                );
                if !surface.content_mask.bounds.contains(&sample)
                    || inside(target, surface.corner_radii, sample)
                {
                    continue;
                }
                for quad in quads.iter().filter(|quad| quad.order >= surface.order) {
                    // A deferred tooltip/popover may legitimately overlap this
                    // surface. It escapes both its bounds and its rectangular
                    // content mask; descendants of a clipped surface cannot.
                    if !contains_bounds(target, quad.bounds)
                        && !contains_bounds(target, quad.content_mask.bounds)
                    {
                        continue;
                    }
                    if paints_at(quad, sample) {
                        return Err(format!(
                            "paint covers curve ({left}, {top}) at {sample:?}: {quad:?}"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn assert_painted_corners(
    window: &Window,
    bounds: Bounds<Pixels>,
    rounded_left: bool,
    name: &str,
) {
    if let Err(error) = check_painted_corners(window, bounds, rounded_left) {
        panic!("{name}: {error}");
    }
}

/// Audit every painted rounded surface within a page's visible viewport. This
/// includes dynamically rendered cards, previews and badges, not a hand-picked
/// list of probes that can miss a newly added component.
pub fn assert_region_corners(window: &Window, region: Bounds<Pixels>, name: &str) {
    let scale = window.scale_factor();
    let region = region.scale(scale);
    for quad in window.painted_quads() {
        // Subpixel canvas dots are glyphs, not nested surfaces. Bars/capsules
        // have their own native fixture; scan badges and larger shapes here.
        if quad.background.is_transparent()
            || quad.corner_radii.top_right.0 < radii::BADGE * scale
            || quad.bounds.left() < region.left()
            || quad.bounds.right() > region.right()
            || quad.bounds.top() < region.top()
            || quad.bounds.bottom() > region.bottom()
        {
            continue;
        }
        let bounds = Bounds::from_corners(
            point(
                px(quad.bounds.left().0 / scale),
                px(quad.bounds.top().0 / scale),
            ),
            point(
                px(quad.bounds.right().0 / scale),
                px(quad.bounds.bottom().0 / scale),
            ),
        );
        assert_painted_corners(window, bounds, quad.corner_radii.top_left.0 > 0., name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn border_only_paint_does_not_count_its_transparent_interior() {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(100.), px(60.))).scale(1.);
        let quad = Quad {
            bounds,
            content_mask: ContentMask { bounds },
            background: transparent_black().into(),
            border_color: black(),
            border_widths: Edges::all(ScaledPixels(1.)),
            corner_radii: Corners::all(ScaledPixels(12.)),
            ..Default::default()
        };
        let sample = |x, y| point(px(x), px(y)).scale(1.);
        assert!(!paints_at(&quad, sample(1., 1.)));
        assert!(paints_at(&quad, sample(0.5, 20.)));
        assert!(!paints_at(&quad, sample(20., 20.)));
        assert!(paints_at(&quad, sample(99.5, 20.)));
    }

    #[test]
    fn corner_containment_is_scale_and_translation_invariant() {
        for scale in [1., 1.25, 1.5, 2., 3.] {
            for offset in [0., 0.5, 19.25] {
                let bounds = Bounds::new(point(px(offset), px(offset)), size(px(100.), px(60.)))
                    .scale(scale);
                let corners = Corners::all(px(12.)).map(|r| r.scale(scale));
                for (x, y, expected) in [
                    (1., 1., false),
                    (5., 5., true),
                    (50., 30., true),
                    (99., 59., false),
                    (95., 55., true),
                ] {
                    let sample = point(px(offset + x), px(offset + y)).scale(scale);
                    assert_eq!(
                        inside(bounds, corners, sample),
                        expected,
                        "scale={scale} offset={offset}"
                    );
                }
            }
        }
    }
}
