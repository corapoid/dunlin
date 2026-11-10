//! Surface geometry is shared by GPUI paint, child edges and native backdrops.
//!
//! GPUI's overflow masks are rectangular, NOT rounded clips. A painted child
//! touching a curved edge must use `round_child`, or stay inside that curve.
//! Do not add padding to every surface: editors, tabs and split joins are flush.
use crate::*;

/// Semantic corner tokens. Keep decorative/dense controls distinct from panels.
pub mod radii {
    pub const SQUARE: f32 = 0.;
    pub const INLINE_ACTION: f32 = 2.;
    pub const BADGE: f32 = 3.;
    pub const CODE: f32 = 4.;
    pub const ICON: f32 = 5.;
    pub const ROW: f32 = 6.;
    pub const CONTROL: f32 = 7.;
    pub const PANEL: f32 = 8.;
    pub const MESSAGE: f32 = 9.;
    pub const DRAG_PREVIEW: f32 = 10.;
    pub const GRAPH_NODE: f32 = 11.;
    pub const CARD: f32 = 12.;
    pub const AUTH_DIALOG: f32 = 14.;
    pub const WORKSPACE: f32 = 16.;
    pub const MODAL: f32 = 20.;

    /// GPUI Kit's folder-tab join; shared by tab bars and matching sidebars.
    pub const FOLDER_TAB: f32 = if CARD < 32. / 3. { CARD } else { 32. / 3. };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceEdges {
    All,
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceGeometry {
    pub radius: f32,
    pub border: f32,
}

impl SurfaceGeometry {
    pub const CARD: Self = Self::new(radii::CARD, 0.);
    pub const DIALOG: Self = Self::new(radii::CARD, 1.);
    pub const SETTINGS: Self = Self::new(radii::MODAL, 1.);
    pub const SETTINGS_CONTENT: Self = Self::new(radii::MODAL - 1., 0.);
    pub const SETTINGS_CARD: Self = Self::new(radii::CARD, 1.);
    pub const CHOICE_CARD: Self = Self::new(radii::CARD, 1.);
    pub const AUTH_DIALOG: Self = Self::new(radii::AUTH_DIALOG, 1.);
    pub const WORKSPACE: Self = Self::new(radii::WORKSPACE, 0.);
    pub const COMPOSER: Self = Self::new(radii::MODAL, 1.);
    pub const TABLE: Self = Self::new(radii::CODE, 1.);
    pub const GRAPH_NODE: Self = Self::new(radii::GRAPH_NODE, 1.);

    pub const fn new(radius: f32, border: f32) -> Self {
        Self { radius, border }
    }

    /// Children start inside the border. Keep the curves concentric there.
    pub fn inner_radius(self) -> f32 {
        (self.radius - self.border).max(0.)
    }

    pub fn paint(self, background: Hsla, border_color: Hsla) -> Div {
        div()
            .rounded(px(self.radius))
            .border(px(self.border))
            .border_color(border_color)
            .bg(background)
    }

    /// Interface surfaces may carry grain; reading/code surfaces use `paint`.
    pub fn paint_material(self, p: Palette, background: Hsla, border_color: Hsla) -> Div {
        self.paint(background, border_color)
            .relative()
            .children(crate::grain_layer(p, self.inner_radius()))
    }

    /// Round only the child's exposed edges; internal joins stay square.
    pub fn round_child<T: Styled>(self, element: T, edges: SurfaceEdges) -> T {
        let radius = px(self.inner_radius());
        element
            .rounded_tl(
                if matches!(
                    edges,
                    SurfaceEdges::All | SurfaceEdges::Top | SurfaceEdges::Left
                ) {
                    radius
                } else {
                    px(0.)
                },
            )
            .rounded_tr(
                if matches!(
                    edges,
                    SurfaceEdges::All | SurfaceEdges::Top | SurfaceEdges::Right
                ) {
                    radius
                } else {
                    px(0.)
                },
            )
            .rounded_bl(
                if matches!(
                    edges,
                    SurfaceEdges::All | SurfaceEdges::Bottom | SurfaceEdges::Left
                ) {
                    radius
                } else {
                    px(0.)
                },
            )
            .rounded_br(
                if matches!(
                    edges,
                    SurfaceEdges::All | SurfaceEdges::Bottom | SurfaceEdges::Right
                ) {
                    radius
                } else {
                    px(0.)
                },
            )
    }

    /// The probe is an absolute child and therefore measures inside the border.
    pub fn outer_bounds(self, inner: Bounds<Pixels>) -> Bounds<Pixels> {
        inner.dilate(px(self.border))
    }

    /// Thin bottom status paint cannot follow a large card curve. Keep only
    /// this decoration between the corner centres; do not inset the whole body.
    pub fn bottom_indicator<T: Styled>(self, element: T) -> T {
        element
            .absolute()
            .bottom_0()
            .left(px(self.inner_radius()))
            .right(px(self.inner_radius()))
    }
}

/// A capsule painted through segment masks, not square children in a rounded div.
/// Every segment paints the SAME rounded quad, clipped to its horizontal slice.
pub fn segmented_bar(segments: impl IntoIterator<Item = (f32, Hsla)>) -> Div {
    let segments: Vec<_> = segments
        .into_iter()
        .filter(|(weight, _)| weight.is_finite() && *weight > 0.)
        .collect();
    div().h(px(4.)).w_full().child(
        canvas(
            |bounds, _, _| bounds,
            move |bounds, _, window, _| {
                let total: f32 = segments.iter().map(|(weight, _)| weight).sum();
                if !total.is_finite() || total <= 0. {
                    return;
                }
                let mut offset = 0.;
                for (index, (weight, color)) in segments.iter().enumerate() {
                    let start = bounds.left() + bounds.size.width * offset;
                    offset += weight / total;
                    let end = if index + 1 == segments.len() {
                        bounds.right()
                    } else {
                        bounds.left() + bounds.size.width * offset
                    };
                    let clip = Bounds::from_corners(
                        point(start, bounds.top()),
                        point(end, bounds.bottom()),
                    );
                    window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                        window.paint_quad(
                            fill(bounds, *color)
                                .corner_radii(Corners::all(bounds.size.height / 2.)),
                        );
                    });
                }
            },
        )
        .size_full(),
    )
}

/// Progress keeps its rounded moving cap, except when the fill is narrower than
/// the bar's height: that sliver is clipped to the track's full capsule instead
/// of painting a small-radius child outside the larger track curve.
pub fn progress_bar(percent: f32, accent: Hsla, track: Hsla) -> Div {
    let fraction = if percent.is_finite() {
        percent.clamp(0., 100.) / 100.
    } else {
        0.
    };
    div().h(px(4.)).w_full().child(
        canvas(
            |bounds, _, _| bounds,
            move |bounds, _, window, _| {
                let corners = Corners::all(bounds.size.height / 2.);
                window.paint_quad(fill(bounds, track).corner_radii(corners));
                if fraction <= 0. {
                    return;
                }
                let filled = Bounds::new(
                    bounds.origin,
                    size(bounds.size.width * fraction, bounds.size.height),
                );
                if filled.size.width >= bounds.size.height {
                    window.paint_quad(fill(filled, accent).corner_radii(corners));
                } else {
                    window.with_content_mask(Some(ContentMask { bounds: filled }), |window| {
                        window.paint_quad(fill(bounds, accent).corner_radii(corners));
                    });
                }
            },
        )
        .size_full(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_curves_and_native_bounds_use_the_same_border_geometry() {
        for geometry in [
            SurfaceGeometry::CARD,
            SurfaceGeometry::DIALOG,
            SurfaceGeometry::SETTINGS,
            SurfaceGeometry::SETTINGS_CONTENT,
            SurfaceGeometry::SETTINGS_CARD,
            SurfaceGeometry::CHOICE_CARD,
            SurfaceGeometry::AUTH_DIALOG,
            SurfaceGeometry::WORKSPACE,
            SurfaceGeometry::COMPOSER,
            SurfaceGeometry::TABLE,
            SurfaceGeometry::GRAPH_NODE,
        ] {
            assert!(geometry.radius >= geometry.border);
            assert_eq!(geometry.inner_radius() + geometry.border, geometry.radius);
            let inner = Bounds::new(point(px(51.), px(31.)), size(px(300.), px(200.)));
            let outer = geometry.outer_bounds(inner);
            assert_eq!(outer.left(), inner.left() - px(geometry.border));
            assert_eq!(
                outer.size.width,
                inner.size.width + px(2. * geometry.border)
            );
            for edges in [
                SurfaceEdges::All,
                SurfaceEdges::Top,
                SurfaceEdges::Bottom,
                SurfaceEdges::Left,
                SurfaceEdges::Right,
            ] {
                let mut child = geometry.round_child(div(), edges);
                let corners = &child.style().corner_radii;
                let expected =
                    |exposed| Some(px(if exposed { geometry.inner_radius() } else { 0. }).into());
                assert_eq!(
                    corners.top_left,
                    expected(matches!(
                        edges,
                        SurfaceEdges::All | SurfaceEdges::Top | SurfaceEdges::Left
                    ))
                );
                assert_eq!(
                    corners.top_right,
                    expected(matches!(
                        edges,
                        SurfaceEdges::All | SurfaceEdges::Top | SurfaceEdges::Right
                    ))
                );
                assert_eq!(
                    corners.bottom_left,
                    expected(matches!(
                        edges,
                        SurfaceEdges::All | SurfaceEdges::Bottom | SurfaceEdges::Left
                    ))
                );
                assert_eq!(
                    corners.bottom_right,
                    expected(matches!(
                        edges,
                        SurfaceEdges::All | SurfaceEdges::Bottom | SurfaceEdges::Right
                    ))
                );
            }
        }
        assert_eq!(SurfaceGeometry::new(1., 2.).inner_radius(), 0.);
    }
}
