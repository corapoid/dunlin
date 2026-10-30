//! One backdrop per window/frame, measured from the foreground surface rather
//! than guessed dialog sizes. No screen capture, private filters or hitboxes.
use dunlin_ui::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    future::Future,
    rc::{Rc, Weak},
};

// Native glass controls its own translucency and blur. Fading the effect view
// mixes sharp lettering back into the result; use the material at full alpha.
const GLASS_ALPHA: f64 = 1.;
const BACKGROUND_BLUR_ALPHA: f64 = 0.14;
const DIM_ALPHA: f32 = 0.12;

#[derive(Clone, Default)]
struct NativeDepth(Rc<Cell<usize>>);
impl NativeDepth {
    fn get(&self) -> usize {
        self.0.get()
    }
    fn acquire(&self) -> DepthLease {
        self.0.set(self.0.get() + 1);
        DepthLease(self.0.clone())
    }
}
struct DepthLease(Rc<Cell<usize>>);
impl Drop for DepthLease {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

#[derive(Default)]
struct Frame {
    surface: Option<Bounds<Pixels>>,
    menu: Option<Bounds<Pixels>>,
    radius: f32,
}
impl Frame {
    fn foreground(&self) -> Option<Bounds<Pixels>> {
        self.menu.or(self.surface)
    }
}

#[derive(Default)]
struct Shared {
    frame: Rc<RefCell<Frame>>,
    native_depth: NativeDepth,
    native: Rc<RefCell<native::BackdropViews>>,
    panel_plane: Cell<bool>,
    external_modal: Cell<bool>,
}
#[derive(Clone, Default)]
pub(crate) struct OverlayBackdrop {
    shared: Rc<Shared>,
}

#[derive(Default)]
struct WindowBackdrops(HashMap<WindowId, Weak<Shared>>);
impl Global for WindowBackdrops {}
impl WindowBackdrops {
    fn controller(&mut self, id: WindowId) -> OverlayBackdrop {
        self.0.retain(|_, controller| controller.strong_count() > 0);
        if let Some(shared) = self.0.get(&id).and_then(Weak::upgrade) {
            return OverlayBackdrop { shared };
        }
        let controller = OverlayBackdrop::default();
        self.0.insert(id, Rc::downgrade(&controller.shared));
        controller
    }
}

pub(crate) fn for_window(window: &mut Window, cx: &mut App) -> OverlayBackdrop {
    // Element-keyed state is namespaced by the current view and would give a
    // deferred menu a different controller from its owning application. Share
    // by WindowId instead, retaining only weak references in the registry.
    if !cx.has_global::<WindowBackdrops>() {
        cx.set_global(WindowBackdrops::default());
    }
    let registry = cx.global_mut::<WindowBackdrops>();
    let id = window.window_handle().window_id();
    registry.controller(id)
}

/// Keep native prompt leases shared but frame measurements/native layers local.
pub(crate) fn for_panel_window(
    window: &mut Window,
    cx: &mut App,
    parent: &OverlayBackdrop,
) -> OverlayBackdrop {
    let controller = panel_controller(parent);
    if !cx.has_global::<WindowBackdrops>() {
        cx.set_global(WindowBackdrops::default());
    }
    cx.global_mut::<WindowBackdrops>().0.insert(
        window.window_handle().window_id(),
        Rc::downgrade(&controller.shared),
    );
    controller
}
fn panel_controller(parent: &OverlayBackdrop) -> OverlayBackdrop {
    OverlayBackdrop {
        shared: Rc::new(Shared {
            native_depth: parent.shared.native_depth.clone(),
            panel_plane: Cell::new(true),
            ..Default::default()
        }),
    }
}

fn dim_color(p: &Palette) -> Hsla {
    Hsla {
        a: DIM_ALPHA,
        ..p.overlay
    }
}

impl OverlayBackdrop {
    pub(crate) fn set_external_modal(&self, active: bool) {
        self.shared.external_modal.set(active);
    }
    pub(crate) fn is_panel_plane(&self) -> bool {
        self.shared.panel_plane.get()
    }
    #[cfg(debug_assertions)]
    pub(crate) fn panel_diagnostic(&self) -> Option<Bounds<Pixels>> {
        self.shared
            .native
            .borrow()
            .panel_ready()
            .then(|| self.shared.frame.borrow().surface)
            .flatten()
    }
    /// First prepaint in the root: also runs when a component-owned menu opens
    /// without changing the application's own modal state.
    pub(crate) fn begin(&self) -> AnyElement {
        let frame = self.shared.frame.clone();
        canvas(
            move |_, _, _| *frame.borrow_mut() = Frame::default(),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .size_full()
        .into_any_element()
    }
    /// Final root paint happens after prepaint of deferred menus. Apply one
    /// dim/blur pass, regardless of how many dialogs are stacked underneath.
    pub(crate) fn finish(&self) -> AnyElement {
        let this = self.clone();
        canvas(
            |bounds, _, _| bounds,
            move |viewport, _, window, cx| {
                let panel_plane = this.shared.panel_plane.get();
                let native_prompt = this.shared.native_depth.get() > 0;
                let foreground = this.shared.frame.borrow().foreground();
                let shape = {
                    let frame = this.shared.frame.borrow();
                    if this.shared.native_depth.get() == 0
                        && !this.shared.external_modal.get()
                        && frame.menu.is_none()
                        && frame.radius > 0.
                    {
                        foreground.map(|bounds| (bounds, frame.radius))
                    } else {
                        None
                    }
                };
                let regions = regions(
                    viewport,
                    foreground,
                    native_prompt || this.shared.external_modal.get(),
                );
                let p = palette(cx);
                if panel_plane && !native_prompt {
                    let frame = this.shared.frame.borrow();
                    // Glass belongs below this plane's controls. The owner has
                    // already been dimmed; do not add a second fullscreen pass.
                    this.shared.native.borrow_mut().update_panel(
                        window,
                        viewport,
                        frame.surface,
                        frame.radius,
                        p.bg.l < 0.5,
                    );
                    return;
                }
                if let Some((foreground, radius)) = shape
                    && let Some(path) = rounded_backdrop_path(viewport, foreground, radius)
                {
                    window.paint_path(path, dim_color(&p));
                } else {
                    for region in &regions {
                        window.paint_quad(fill(*region, dim_color(&p)));
                    }
                }
                this.shared.native.borrow_mut().update(
                    window,
                    viewport,
                    &regions,
                    p.bg.l < 0.5,
                    shape,
                );
            },
        )
        .absolute()
        .inset_0()
        .size_full()
        .into_any_element()
    }
    /// Keep native confirmations/file sheets visually consistent without
    /// replacing their buttons, changing answers or weakening safety gates.
    pub(crate) fn track_native<F: Future + 'static>(
        &self,
        future: F,
        cx: &mut App,
    ) -> Task<F::Output>
    where
        F::Output: 'static,
    {
        let lease = NativeLease {
            _depth: self.shared.native_depth.acquire(),
            cx: cx.to_async(),
        };
        cx.refresh_windows();
        cx.spawn(async move |_| {
            let _lease = lease;
            future.await
        })
    }
}

struct NativeLease {
    _depth: DepthLease,
    cx: AsyncApp,
}
impl Drop for NativeLease {
    fn drop(&mut self) {
        // A task can be cancelled from inside a UI update; never reborrow App
        // synchronously during Drop. The executor stops when App is released.
        self.cx
            .spawn(async move |cx| cx.update(|cx| cx.refresh_windows()))
            .detach();
    }
}

/// Attach inside the actual foreground panel, not the full-screen dismiss layer.
pub(crate) fn rounded_probe(backdrop: &OverlayBackdrop, radius: f32) -> AnyElement {
    surface_probe(backdrop, false, radius)
}
/// A scroll container translates all its children, including an absolute
/// measuring canvas. Its content mask, unlike child coordinates, stays fixed
/// to the visible panel; do not move the sharp cutout when its contents scroll.
pub(crate) fn rounded_scroll_probe(backdrop: &OverlayBackdrop, radius: f32) -> AnyElement {
    surface_probe(backdrop, true, radius)
}
fn surface_probe(backdrop: &OverlayBackdrop, scroll: bool, radius: f32) -> AnyElement {
    let frame = backdrop.shared.frame.clone();
    canvas(
        move |bounds, window, _| {
            let bounds = surface_bounds(bounds, window.content_mask().bounds, scroll);
            if nonempty(bounds) {
                frame.borrow_mut().surface = Some(bounds);
                frame.borrow_mut().radius = radius;
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
    .size_full()
    .into_any_element()
}

fn surface_bounds(child: Bounds<Pixels>, clip: Bounds<Pixels>, scroll: bool) -> Bounds<Pixels> {
    // Every measured modal has a 1-point border. Absolute children and scroll
    // clips are measured inside that border; native glass must include it or
    // its corners end up inset by one point on every edge.
    (if scroll { clip } else { child }).dilate(px(1.))
}

/// Component menus are deferred above dialogs. Observe their visible custom
/// rows, including the menu's icon/check/padding gutter and shadow, so opening
/// a dropdown over Settings never blurs the menu itself. Preserve submenu chains.
pub(crate) fn menu_surface<T: ParentElement>(content: T, window: &mut Window, cx: &mut App) -> T {
    let frame = for_window(window, cx).shared.frame.clone();
    content.child(
        canvas(
            move |bounds, window, _| {
                let visible = bounds.intersect(&window.content_mask().bounds);
                if nonempty(visible) {
                    let visible = visible.dilate(px(32.));
                    let mut frame = frame.borrow_mut();
                    frame.menu = Some(
                        frame
                            .menu
                            .map(|rect| rect.union(&visible))
                            .unwrap_or(visible),
                    );
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .size_full(),
    )
}

pub(crate) fn menu_item<F, E>(render: F) -> component::menu::PopupMenuItem
where
    F: Fn(&mut Window, &mut App) -> E + 'static,
    E: ParentElement + IntoElement + 'static,
{
    component::menu::PopupMenuItem::element(move |window, cx| {
        let content = render(window, cx);
        menu_surface(content, window, cx)
    })
}

fn nonempty(bounds: Bounds<Pixels>) -> bool {
    [
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    ]
    .iter()
    .all(|v| v.is_finite())
        && bounds.size.width > px(0.)
        && bounds.size.height > px(0.)
}
fn regions(
    viewport: Bounds<Pixels>,
    foreground: Option<Bounds<Pixels>>,
    native: bool,
) -> Vec<Bounds<Pixels>> {
    if !nonempty(viewport) {
        return Vec::new();
    }
    if native {
        return vec![viewport];
    }
    let Some(foreground) = foreground
        .filter(|r| nonempty(*r))
        .map(|r| r.intersect(&viewport))
        .filter(|r| nonempty(*r))
    else {
        return Vec::new();
    };
    let (x, y) = (viewport.left(), viewport.top());
    let (right, bottom) = (viewport.right(), viewport.bottom());
    let (left, top) = (foreground.left(), foreground.top());
    let (end_x, end_y) = (foreground.right(), foreground.bottom());
    [
        Bounds::new(point(x, y), size(right - x, top - y)),
        Bounds::new(point(x, end_y), size(right - x, bottom - end_y)),
        Bounds::new(point(x, top), size(left - x, end_y - top)),
        Bounds::new(point(end_x, top), size(right - end_x, end_y - top)),
    ]
    .into_iter()
    .filter(|r| nonempty(*r))
    .collect()
}

/// The dim and native blur need the same rounded hole. A rectangular dim cutout
/// leaves bright corner triangles even when the native blur mask is rounded.
/// Opposite contour winding works with both even-odd and nonzero tessellators.
fn rounded_backdrop_path(
    viewport: Bounds<Pixels>,
    foreground: Bounds<Pixels>,
    radius: f32,
) -> Option<Path<Pixels>> {
    if !nonempty(viewport) || !nonempty(foreground) || !radius.is_finite() {
        return None;
    }
    let mut path = PathBuilder::fill();
    path.move_to(viewport.origin);
    path.line_to(point(viewport.right(), viewport.top()));
    path.line_to(point(viewport.right(), viewport.bottom()));
    path.line_to(point(viewport.left(), viewport.bottom()));
    path.close();
    let r = px(radius
        .max(0.)
        .min(f32::from(foreground.size.width) / 2.)
        .min(f32::from(foreground.size.height) / 2.));
    let (left, top, right, bottom) = (
        foreground.left(),
        foreground.top(),
        foreground.right(),
        foreground.bottom(),
    );
    path.move_to(point(left + r, top));
    path.arc_to(point(r, r), px(0.), false, false, point(left, top + r));
    path.line_to(point(left, bottom - r));
    path.arc_to(point(r, r), px(0.), false, false, point(left + r, bottom));
    path.line_to(point(right - r, bottom));
    path.arc_to(point(r, r), px(0.), false, false, point(right, bottom - r));
    path.line_to(point(right, top + r));
    path.arc_to(point(r, r), px(0.), false, false, point(right - r, top));
    path.close();
    path.build().ok()
}

/// AppKit points, not backing pixels. GPUI's logical viewport may have a
/// different size (UI zoom); never put a logical-sized bitmap into a native
/// effect view and let NSImage's mask/nine-slice machinery rescale it again.
#[derive(Clone, Copy, Debug, PartialEq)]
struct NativeRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
impl NativeRect {
    fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.width > 0.
            && self.height > 0.
    }
}
fn native_rect(
    viewport: Bounds<Pixels>,
    native: NativeRect,
    region: Bounds<Pixels>,
    flipped: bool,
) -> Option<NativeRect> {
    if !nonempty(viewport) || !native.valid() || !nonempty(region) {
        return None;
    }
    let sx = native.width / f64::from(f32::from(viewport.size.width));
    let sy = native.height / f64::from(f32::from(viewport.size.height));
    let x = f64::from(f32::from(region.left() - viewport.left())) * sx;
    let y = f64::from(f32::from(region.top() - viewport.top())) * sy;
    let height = f64::from(f32::from(region.size.height)) * sy;
    Some(NativeRect {
        x: native.x + x,
        y: native.y
            + if flipped {
                y
            } else {
                native.height - y - height
            },
        width: f64::from(f32::from(region.size.width)) * sx,
        height,
    })
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use objc2::{
        MainThreadMarker, MainThreadOnly, define_class, extern_class, extern_methods, msg_send,
        rc::Retained, runtime::AnyClass, sel,
    };
    use objc2_app_kit::{
        NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
        NSBezierPath, NSResponder, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
        NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
    };
    use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize};
    use objc2_quartz_core::{CAShapeLayer, CATransaction, kCAFillRuleEvenOdd};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    define_class!(
        // SAFETY: Main-thread-only AppKit subclass, no ivars/custom destruction.
        // hitTest: preserves NSView's ABI and deliberately passes every event on.
        #[unsafe(super = NSVisualEffectView)]
        #[thread_kind = MainThreadOnly]
        struct Backdrop;
        impl Backdrop {
            #[unsafe(method(hitTest:))]
            fn hit_test(&self, _: NSPoint) -> *mut NSView { std::ptr::null_mut() }
        }
    );

    // Public macOS 26+ API. Our objc2-app-kit version predates this binding.
    // Resolve availability before touching this class or registering its child,
    // so the same executable still runs on the supported macOS 15 baseline.
    // API/ABI: AppKit NSGlassEffectView.h (NSInteger style, CGFloat radius).
    extern_class!(
        #[unsafe(super(NSView, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "NSGlassEffectView"]
        struct GlassEffectView;
    );
    impl GlassEffectView {
        extern_methods!(
            #[unsafe(method(setStyle:))]
            #[unsafe(method_family = none)]
            fn set_style(&self, style: isize);
            #[unsafe(method(setCornerRadius:))]
            #[unsafe(method_family = none)]
            fn set_corner_radius(&self, radius: f64);
            #[unsafe(method(setContentView:))]
            #[unsafe(method_family = none)]
            fn set_content_view(&self, content: Option<&NSView>);
            #[unsafe(method(contentView))]
            #[unsafe(method_family = none)]
            fn content_view(&self) -> Option<Retained<NSView>>;
        );
    }
    define_class!(
        // SAFETY: Public AppKit subclass, main-thread-only, unchanged hitTest:
        // ABI. It has no interactive content and never steals app input.
        #[unsafe(super = GlassEffectView)]
        #[thread_kind = MainThreadOnly]
        struct GlassBackdrop;
        impl GlassBackdrop {
            #[unsafe(method(hitTest:))]
            fn hit_test(&self, _: NSPoint) -> *mut NSView { std::ptr::null_mut() }
        }
    );
    const GLASS_STYLE_REGULAR: isize = 0; // NSGlassEffectViewStyleRegular, public SDK enum.
    const FROSTED_MATERIAL: NSVisualEffectMaterial = NSVisualEffectMaterial::Popover;
    fn frosted_profile(panel: bool) -> (NSVisualEffectMaterial, NSVisualEffectBlendingMode, f64) {
        if panel {
            (
                FROSTED_MATERIAL,
                NSVisualEffectBlendingMode::BehindWindow,
                GLASS_ALPHA,
            )
        } else {
            (
                NSVisualEffectMaterial::HeaderView,
                NSVisualEffectBlendingMode::WithinWindow,
                BACKGROUND_BLUR_ALPHA,
            )
        }
    }
    fn liquid_glass_available() -> bool {
        supports_liquid_glass(AnyClass::get(c"NSGlassEffectView"))
    }
    fn supports_liquid_glass(class: Option<&AnyClass>) -> bool {
        class.is_some_and(|class| {
            [
                sel!(setStyle:),
                sel!(setCornerRadius:),
                sel!(setContentView:),
                sel!(contentView),
            ]
            .iter()
            .all(|selector| class.instance_method(*selector).is_some())
        })
    }
    enum Surface {
        Liquid(Retained<GlassBackdrop>),
        Frosted(Retained<Backdrop>),
    }
    impl Surface {
        fn new(marker: MainThreadMarker, panel: bool) -> Self {
            let surface = if panel && liquid_glass_available() {
                crate::logging::info("native panel material: Liquid Glass regular");
                // SAFETY: Runtime class/selector availability was checked above;
                // initWithFrame: follows NSView's retained initializer contract.
                let view: Retained<GlassBackdrop> =
                    unsafe { msg_send![GlassBackdrop::alloc(marker), initWithFrame: NSRect::ZERO] };
                view.set_style(GLASS_STYLE_REGULAR);
                // The rounded foreground hole is applied separately. Rounding
                // the full-window effect would expose the old sharp corners.
                view.set_corner_radius(0.);
                // Only contentView has a documented z-order inside glass. Keep
                // it transparent/empty; GPUI text stays outside the glass mask.
                let content = NSView::initWithFrame(NSView::alloc(marker), NSRect::ZERO);
                view.set_content_view(Some(&content));
                Self::Liquid(view)
            } else {
                crate::logging::info(if panel {
                    "native panel material: AppKit Popover fallback"
                } else {
                    "native owner backdrop: subtle HeaderView blur"
                });
                let view: Retained<Backdrop> =
                    unsafe { msg_send![Backdrop::alloc(marker), initWithFrame: NSRect::ZERO] };
                let (material, blend, _) = frosted_profile(panel);
                view.setBlendingMode(blend);
                // Popover is a translucent overlay material. UnderWindowBackground
                // is not a substitute for frosted glass over in-window content.
                view.setMaterial(material);
                view.setState(NSVisualEffectState::Active);
                Self::Frosted(view)
            };
            surface.view().setWantsLayer(true);
            surface.view().setAlphaValue(if panel {
                GLASS_ALPHA
            } else {
                BACKGROUND_BLUR_ALPHA
            });
            surface
        }
        fn view(&self) -> &NSView {
            match self {
                Self::Liquid(view) => view,
                Self::Frosted(view) => view,
            }
        }
        fn layout_content(&self) {
            if let Self::Liquid(view) = self
                && let Some(content) = view.content_view()
            {
                content.setFrame(view.bounds());
            }
        }
        fn set_radius(&self, radius: f64) {
            if let Self::Liquid(view) = self {
                view.set_corner_radius(radius);
            }
            if let Some(layer) = self.view().layer() {
                layer.setCornerRadius(radius);
                layer.setMasksToBounds(true);
            }
        }
    }
    #[derive(Default)]
    pub(super) struct BackdropViews {
        views: Vec<Surface>,
        last: Option<Geometry>,
        panel_surface: Option<Surface>,
        last_panel: Option<Geometry>,
        #[cfg(debug_assertions)]
        panel_root: usize,
    }
    #[derive(PartialEq)]
    struct Geometry {
        viewport: Bounds<Pixels>,
        regions: Vec<Bounds<Pixels>>,
        dark: bool,
        shape: Option<(Bounds<Pixels>, f32)>,
        native_bounds: NativeRect,
        flipped: bool,
        scale: f64,
    }
    fn rect(bounds: NSRect) -> NativeRect {
        NativeRect {
            x: bounds.origin.x,
            y: bounds.origin.y,
            width: bounds.size.width,
            height: bounds.size.height,
        }
    }
    fn ns_rect(bounds: NativeRect) -> NSRect {
        NSRect::new(
            NSPoint::new(bounds.x, bounds.y),
            NSSize::new(bounds.width, bounds.height),
        )
    }
    impl BackdropViews {
        #[cfg(debug_assertions)]
        pub(super) fn panel_ready(&self) -> bool {
            let Some(surface) = &self.panel_surface else {
                return false;
            };
            if self.last_panel.is_none() {
                return false;
            }
            let view = surface.view();
            // SAFETY: Diagnostics run on the main thread while this retained
            // surface and its host window are alive. No capture or UI mutation.
            let Some(host) = (unsafe { view.superview() }) else {
                return false;
            };
            let children = host.subviews();
            let glass_index = children.iter().position(|v| std::ptr::eq(&*v, view));
            let controls_index = children
                .iter()
                .position(|v| std::ptr::from_ref(&*v) as usize == self.panel_root);
            matches!((glass_index, controls_index), (Some(glass), Some(controls)) if glass < controls)
        }
        pub(super) fn update_panel(
            &mut self,
            window: &Window,
            viewport: Bounds<Pixels>,
            foreground: Option<Bounds<Pixels>>,
            radius: f32,
            dark: bool,
        ) {
            let Some(foreground) = foreground.filter(|r| nonempty(*r)) else {
                self.clear();
                return;
            };
            if !radius.is_finite() {
                self.clear();
                return;
            }
            for view in self.views.drain(..) {
                view.view().removeFromSuperview();
            }
            self.last = None;
            let Some(marker) = MainThreadMarker::new() else {
                return;
            };
            let Ok(handle) = HasWindowHandle::window_handle(window) else {
                return;
            };
            let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                return;
            };
            // SAFETY: GPUI owns this live view; the new native surface is a
            // sibling BELOW its Metal drawing layer, never an overlay on text.
            let root: &NSView = unsafe { &*handle.ns_view.as_ptr().cast() };
            #[cfg(debug_assertions)]
            {
                self.panel_root = std::ptr::from_ref(root) as usize;
            }
            // SAFETY: The attached root and its live superview are borrowed on
            // the main thread and no hierarchy mutation occurs during lookup.
            let Some(host) = (unsafe { root.superview() }) else {
                return;
            };
            let bounds = rect(root.bounds());
            let Some(frame) = native_rect(viewport, bounds, foreground, root.isFlipped()) else {
                self.clear();
                return;
            };
            let scale = root
                .window()
                .map(|w| w.backingScaleFactor())
                .unwrap_or(f64::from(window.scale_factor()));
            let geometry = Geometry {
                viewport,
                regions: vec![foreground],
                dark,
                shape: Some((foreground, radius)),
                native_bounds: bounds,
                flipped: root.isFlipped(),
                scale,
            };
            if self.last_panel.as_ref() == Some(&geometry) {
                return;
            }
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            let surface = self.panel_surface.get_or_insert_with(|| {
                let surface = Surface::new(marker, true);
                host.addSubview_positioned_relativeTo(
                    surface.view(),
                    NSWindowOrderingMode::Below,
                    Some(root),
                );
                surface
            });
            let view = surface.view();
            view.setFrame(host.convertRect_fromView(ns_rect(frame), Some(root)));
            surface.layout_content();
            surface.set_radius(
                f64::from(radius.max(0.)) * bounds.width
                    / f64::from(f32::from(viewport.size.width)),
            );
            let appearance = NSAppearance::appearanceNamed(unsafe {
                if dark {
                    NSAppearanceNameDarkAqua
                } else {
                    NSAppearanceNameAqua
                }
            });
            view.setAppearance(appearance.as_deref());
            CATransaction::commit();
            self.last_panel = Some(geometry);
        }
        pub(super) fn update(
            &mut self,
            window: &Window,
            viewport: Bounds<Pixels>,
            regions: &[Bounds<Pixels>],
            dark: bool,
            shape: Option<(Bounds<Pixels>, f32)>,
        ) {
            if let Some(surface) = self.panel_surface.take() {
                surface.view().removeFromSuperview();
            }
            self.last_panel = None;
            if regions.is_empty() {
                self.clear();
                return;
            }
            let Some(marker) = MainThreadMarker::new() else {
                return;
            };
            let Ok(handle) = HasWindowHandle::window_handle(window) else {
                return;
            };
            let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                return;
            };
            // SAFETY: The raw handle borrows GPUI's live NSView only for this
            // call on its main thread. All installed subviews are owned here.
            let parent: &NSView = unsafe { &*handle.ns_view.as_ptr().cast() };
            let bounds = rect(parent.bounds());
            if !bounds.valid() || !nonempty(viewport) {
                self.clear();
                return;
            }
            let scale = parent
                .window()
                .map(|w| w.backingScaleFactor())
                .unwrap_or(f64::from(window.scale_factor()));
            let geometry = Geometry {
                viewport,
                regions: regions.to_vec(),
                dark,
                shape,
                native_bounds: bounds,
                flipped: parent.isFlipped(),
                scale,
            };
            if self.last.as_ref() == Some(&geometry) {
                return;
            }
            let actual = if shape.is_some() {
                vec![viewport]
            } else {
                regions.to_vec()
            };
            let regions = actual.as_slice();
            // Native layer changes must be immediate, including the first open
            // and moves between displays. Do not animate an old foreground hole.
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            while self.views.len() > regions.len() {
                if let Some(view) = self.views.pop() {
                    view.view().removeFromSuperview();
                }
            }
            while self.views.len() < regions.len() {
                let view = Surface::new(marker, false);
                // Both native APIs honor Reduce Transparency themselves. No
                // screenshot, private blur filter or global preference override.
                parent.addSubview(view.view());
                self.views.push(view);
            }
            let appearance = NSAppearance::appearanceNamed(unsafe {
                if dark {
                    NSAppearanceNameDarkAqua
                } else {
                    NSAppearanceNameAqua
                }
            });
            for (surface, region) in self.views.iter().zip(regions) {
                let view = surface.view();
                let Some(frame) = native_rect(viewport, bounds, *region, parent.isFlipped()) else {
                    continue;
                };
                view.setAppearance(appearance.as_deref());
                view.setFrame(ns_rect(frame));
                surface.layout_content();
                view.setHidden(false);
                if let Some(layer) = view.layer() {
                    let mask = shape.and_then(|(foreground, radius)| {
                        vector_mask(
                            viewport,
                            rect(view.bounds()),
                            foreground,
                            radius,
                            view.isFlipped(),
                            scale,
                        )
                    });
                    // SAFETY: A freshly created, unattached mask layer is owned
                    // by this view only; nil explicitly clears it on menus or
                    // native sheets. The GPUI drawing layer is never masked.
                    unsafe {
                        layer.setMask(mask.as_deref().map(|mask| &**mask));
                    }
                    // A missing/invalid native mask must never turn a rounded
                    // overlay into an opaque full-window material covering it.
                    if shape.is_some() && mask.is_none() {
                        view.setHidden(true);
                    }
                } else if shape.is_some() {
                    view.setHidden(true);
                }
            }
            CATransaction::commit();
            self.last = Some(geometry);
        }
        fn clear(&mut self) {
            self.last = None;
            self.last_panel = None;
            if let Some(surface) = self.panel_surface.take() {
                surface.view().removeFromSuperview();
            }
            for view in self.views.drain(..) {
                view.view().removeFromSuperview();
            }
        }
    }
    impl Drop for BackdropViews {
        fn drop(&mut self) {
            self.clear();
        }
    }
    fn vector_mask(
        viewport: Bounds<Pixels>,
        native: NativeRect,
        foreground: Bounds<Pixels>,
        radius: f32,
        flipped: bool,
        scale: f64,
    ) -> Option<Retained<CAShapeLayer>> {
        if !radius.is_finite() {
            return None;
        }
        let local = NativeRect {
            x: 0.,
            y: 0.,
            ..native
        };
        let hole = native_rect(viewport, local, foreground, flipped)?;
        let rx = (f64::from(radius.max(0.)) * native.width
            / f64::from(f32::from(viewport.size.width)))
        .min(hole.width / 2.);
        let ry = (f64::from(radius.max(0.)) * native.height
            / f64::from(f32::from(viewport.size.height)))
        .min(hole.height / 2.);
        let path = NSBezierPath::bezierPathWithRect(ns_rect(local));
        path.appendBezierPathWithRoundedRect_xRadius_yRadius(ns_rect(hole), rx, ry);
        let mask = CAShapeLayer::layer();
        mask.setFrame(ns_rect(native));
        mask.setContentsScale(scale);
        mask.setFillRule(unsafe { kCAFillRuleEvenOdd });
        mask.setPath(Some(&path.CGPath()));
        Some(mask)
    }

    #[cfg(test)]
    mod tests {
        use super::{Geometry, NativeRect, vector_mask};
        use dunlin_ui::{Bounds, Pixels, point, px, size};
        use objc2_app_kit::{NSBezierPath, NSWindingRule};
        use objc2_foundation::NSPoint;
        fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
            Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
        }
        #[test]
        fn liquid_glass_checks_runtime_capability_before_using_the_new_class() {
            use objc2::{runtime::AnyClass, sel};
            assert!(!super::supports_liquid_glass(None));
            assert!(!super::supports_liquid_glass(AnyClass::get(c"NSView")));
            if let Some(class) = AnyClass::get(c"NSGlassEffectView") {
                assert!(super::liquid_glass_available());
                // Validate the small hand-written public binding against the
                // actual OS runtime, without instantiating a visible window.
                for (selector, argument) in [
                    (sel!(setStyle:), "q"),
                    (sel!(setCornerRadius:), "d"),
                    (sel!(setContentView:), "@"),
                ] {
                    let method = class.instance_method(selector).unwrap();
                    assert_eq!(method.arguments_count(), 3);
                    assert_eq!(method.return_type().to_bytes(), b"v");
                    let encoding = method.argument_type(2).unwrap();
                    if argument.starts_with('@') {
                        assert!(encoding.to_bytes().starts_with(b"@"));
                    } else {
                        assert_eq!(encoding.to_bytes(), argument.as_bytes());
                    }
                }
            }
        }
        #[test]
        fn older_macos_uses_translucent_popover_material_without_alpha_crossfade() {
            use objc2_app_kit::NSVisualEffectMaterial;
            assert_eq!(super::FROSTED_MATERIAL, NSVisualEffectMaterial::Popover);
            assert_ne!(
                super::FROSTED_MATERIAL,
                NSVisualEffectMaterial::UnderWindowBackground
            );
            assert_ne!(
                super::FROSTED_MATERIAL,
                NSVisualEffectMaterial::WindowBackground
            );
            assert_ne!(
                super::FROSTED_MATERIAL,
                NSVisualEffectMaterial::ContentBackground
            );
            assert_eq!(super::GLASS_ALPHA, 1.);
            assert_eq!(super::GLASS_STYLE_REGULAR, 0);
        }
        #[test]
        fn owner_blur_and_localized_panel_material_are_separate_profiles() {
            use objc2_app_kit::{NSVisualEffectBlendingMode, NSVisualEffectMaterial};
            let (material, blend, alpha) = super::frosted_profile(false);
            assert_eq!(material, NSVisualEffectMaterial::HeaderView);
            assert_eq!(blend, NSVisualEffectBlendingMode::WithinWindow);
            assert!((0.1..=0.2).contains(&alpha));
            let (material, blend, alpha) = super::frosted_profile(true);
            assert_eq!(material, NSVisualEffectMaterial::Popover);
            assert_eq!(blend, NSVisualEffectBlendingMode::BehindWindow);
            assert_eq!(alpha, 1.);
        }
        #[test]
        fn vector_cutout_matches_all_settings_edges_at_retina_and_zoom_scales() {
            let viewport = rect(0., 0., 1680., 1024.);
            let panel = rect(280., 132., 1120., 760.);
            for ratio in [0.6, 1., 1.25, 2.] {
                for scale in [1., 2., 3.] {
                    for flipped in [false, true] {
                        let native = NativeRect {
                            x: 0.,
                            y: 0.,
                            width: 1680. * ratio,
                            height: 1024. * ratio,
                        };
                        let mask =
                            vector_mask(viewport, native, panel, 14., flipped, scale).unwrap();
                        assert_eq!(mask.frame().size.width, native.width);
                        assert_eq!(mask.contentsScale(), scale);
                        let path = NSBezierPath::bezierPathWithCGPath(&mask.path().unwrap());
                        path.setWindingRule(NSWindingRule::EvenOdd);
                        // Sample the whole measured foreground, not only its
                        // center: the bitmap regression left just one quadrant
                        // visible. Rounding must not cover navigation or content.
                        for x in [296., 500., 840., 1200., 1384.] {
                            for y in [148., 300., 512., 760., 876.] {
                                let ny = if flipped { y } else { 1024. - y };
                                assert!(
                                    !path.containsPoint(NSPoint::new(x * ratio, ny * ratio)),
                                    "foreground covered: ratio={ratio} scale={scale} flipped={flipped} x={x} y={y}"
                                );
                            }
                        }
                        assert!(path.containsPoint(NSPoint::new(10. * ratio, 10. * ratio)));
                        let corner_y = if flipped { 132.5 } else { 1024. - 132.5 };
                        assert!(path.containsPoint(NSPoint::new(280.5 * ratio, corner_y * ratio)));
                    }
                }
            }
        }
        #[test]
        fn native_geometry_cache_invalidates_on_resize_scale_flip_and_close() {
            let viewport = rect(0., 0., 1360., 860.);
            let make = || Geometry {
                viewport,
                regions: Vec::new(),
                dark: false,
                shape: Some((rect(120., 50., 1120., 760.), 14.)),
                native_bounds: NativeRect {
                    x: 0.,
                    y: 0.,
                    width: 1360.,
                    height: 860.,
                },
                flipped: false,
                scale: 2.,
            };
            assert!(make() == make());
            let mut resized = make();
            resized.native_bounds.width = 1500.;
            assert!(resized != make());
            let mut display = make();
            display.scale = 1.;
            assert!(display != make());
            let mut flipped = make();
            flipped.flipped = true;
            assert!(flipped != make());
            let mut menu = make();
            menu.shape = None;
            assert!(menu != make());
        }
        #[test]
        fn core_animation_composites_the_whole_foreground_hole_without_image_resizing() {
            use objc2::AnyThread;
            use objc2_app_kit::{
                NSBitmapImageRep, NSColor, NSDeviceRGBColorSpace, NSGraphicsContext,
            };
            use objc2_quartz_core::CALayer;
            for scale in [1., 2.] {
                let native = NativeRect {
                    x: 0.,
                    y: 0.,
                    width: 200.,
                    height: 120.,
                };
                let viewport = rect(0., 0., 400., 240.);
                let mask = vector_mask(
                    viewport,
                    native,
                    rect(88., 44., 224., 152.),
                    14.,
                    false,
                    scale,
                )
                .unwrap();
                let layer = CALayer::layer();
                layer.setFrame(super::ns_rect(native));
                layer.setBackgroundColor(Some(&NSColor::blackColor().CGColor()));
                // SAFETY: The new mask has no parent and cannot form a cycle.
                unsafe {
                    layer.setMask(Some(&mask));
                }
                let width = (200. * scale) as isize;
                let height = (120. * scale) as isize;
                // SAFETY: Null planes requests AppKit-owned bounded RGBA storage.
                let bitmap = unsafe { NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(NSBitmapImageRep::alloc(), std::ptr::null_mut(), width, height, 8, 4, true, false, NSDeviceRGBColorSpace, width * 4, 32) }.unwrap();
                let data = bitmap.bitmapData();
                assert!(!data.is_null());
                // SAFETY: AppKit allocated bytesPerRow * pixelsHigh bytes above.
                unsafe {
                    std::ptr::write_bytes(data, 0, (bitmap.bytesPerRow() * height) as usize);
                }
                let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)
                    .unwrap()
                    .CGContext();
                objc2_core_graphics::CGContext::scale_ctm(Some(&context), scale, scale);
                layer.renderInContext(&context);
                let alpha = |x: f64, y: f64| {
                    bitmap
                        .colorAtX_y((x * scale) as isize, (y * scale) as isize)
                        .unwrap()
                        .alphaComponent()
                };
                assert!(alpha(2., 2.) > 0.95);
                for x in [48., 100., 152.] {
                    for y in [26., 60., 94.] {
                        assert!(
                            alpha(x, y) < 0.05,
                            "foreground alpha at {x},{y} scale {scale}"
                        );
                    }
                }
                assert!(alpha(44., 22.) > 0.5);
                // Clearing the mask for a subsequent native prompt removes the
                // old cutout instead of leaving a transparent orphan region.
                unsafe {
                    layer.setMask(None);
                }
                layer.renderInContext(&context);
                assert!(alpha(100., 60.) > 0.95);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Frame, NativeDepth, regions};
    use dunlin_ui::{Bounds, Pixels, point, px, size};
    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }
    fn area(rect: Bounds<Pixels>) -> f32 {
        f32::from(rect.size.width) * f32::from(rect.size.height)
    }
    #[test]
    fn panel_plane_has_isolated_measurements_and_shared_native_prompt_leases() {
        let parent = super::OverlayBackdrop::default();
        let panel = super::panel_controller(&parent);
        assert!(!parent.is_panel_plane());
        assert!(panel.is_panel_plane());
        parent.shared.frame.borrow_mut().surface = Some(rect(0., 0., 1000., 800.));
        panel.shared.frame.borrow_mut().surface = Some(rect(200., 200., 530., 460.));
        assert_ne!(
            parent.shared.frame.borrow().surface,
            panel.shared.frame.borrow().surface
        );
        let lease = parent.shared.native_depth.acquire();
        assert_eq!(panel.shared.native_depth.get(), 1);
        drop(lease);
        assert_eq!(panel.shared.native_depth.get(), 0);
        for p in [dunlin_ui::theme::light(), dunlin_ui::theme::dark()] {
            assert_eq!(crate::effects::panel_fill(&parent, p.card), p.card);
            assert!((crate::effects::panel_fill(&panel, p.card).a - 0.14).abs() < 0.001);
            assert_eq!(crate::effects::panel_border(&parent, &p), p.border_strong);
        }
    }
    #[test]
    fn measurement_includes_the_border_without_resizing_the_actual_panels() {
        for (w, h) in [(530., 460.), (300., 170.), (1120., 760.)] {
            let interior = rect(101., 201., w - 2., h - 2.);
            assert_eq!(
                super::surface_bounds(interior, interior, false),
                rect(100., 200., w, h)
            );
            assert_eq!(
                super::surface_bounds(interior, interior, true),
                rect(100., 200., w, h)
            );
        }
    }
    fn triangle_coverage(path: &dunlin_ui::Path<Pixels>, x: f32, y: f32) -> usize {
        path.vertices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|triangle| {
                let side = |a: usize, b: usize| {
                    let a = triangle[a].xy_position;
                    let b = triangle[b].xy_position;
                    (x - f32::from(b.x)) * f32::from(a.y - b.y)
                        - f32::from(a.x - b.x) * (y - f32::from(b.y))
                };
                let sides = [side(0, 1), side(1, 2), side(2, 0)];
                sides.iter().all(|s| *s >= 0.) || sides.iter().all(|s| *s <= 0.)
            })
            .count()
    }
    #[test]
    fn rounded_dimming_covers_corner_triangles_without_covering_or_double_dimming_the_panel() {
        let viewport = rect(0., 0., 200., 160.);
        let panel = rect(20., 30., 160., 100.);
        for radius in [8., 12., 14.] {
            let path = super::rounded_backdrop_path(viewport, panel, radius).unwrap();
            for (x, y) in [
                (20.25, 30.25),
                (179.75, 30.25),
                (20.25, 129.75),
                (179.75, 129.75),
                (1.25, 1.75),
                (190.25, 145.75),
            ] {
                assert_eq!(
                    triangle_coverage(&path, x, y),
                    1,
                    "missing or doubled corner dim at {x},{y}, radius={radius}"
                );
            }
            for (x, y) in [
                (20.25, 80.75),
                (179.75, 80.25),
                (100.25, 30.25),
                (100.25, 129.75),
                (60.25, 70.75),
                (140.25, 100.75),
            ] {
                assert_eq!(
                    triangle_coverage(&path, x, y),
                    0,
                    "panel content dimmed at {x},{y}"
                );
            }
            let painted_area: f32 = path
                .vertices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|triangle| {
                    let a = triangle[0].xy_position;
                    let b = triangle[1].xy_position;
                    let c = triangle[2].xy_position;
                    (f32::from(b.x - a.x) * f32::from(c.y - a.y)
                        - f32::from(c.x - a.x) * f32::from(b.y - a.y))
                    .abs()
                        / 2.
                })
                .sum();
            let expected =
                area(viewport) - area(panel) + (4. - std::f32::consts::PI) * radius * radius;
            assert!(
                (painted_area - expected).abs() < 8.,
                "unexpected dim area: {painted_area} vs {expected}"
            );
        }
    }
    #[test]
    fn rounded_dimming_handles_offsets_square_holes_and_invalid_geometry() {
        let viewport = rect(20., 10., 200., 160.);
        let panel = rect(40., 40., 160., 100.);
        let path = super::rounded_backdrop_path(viewport, panel, 0.).unwrap();
        assert_eq!(triangle_coverage(&path, 40.25, 40.25), 0);
        assert_eq!(triangle_coverage(&path, 25.25, 15.75), 1);
        assert!(super::rounded_backdrop_path(viewport, panel, f32::NAN).is_none());
        assert!(super::rounded_backdrop_path(rect(0., 0., 0., 0.), panel, 12.).is_none());
        assert!(
            super::rounded_backdrop_path(viewport, rect(f32::NAN, 0., 10., 10.), 12.).is_none()
        );
    }
    #[test]
    fn logical_regions_map_to_native_points_with_independent_scales_and_flipping() {
        use super::{NativeRect, native_rect};
        let viewport = rect(17., 23., 2000., 1200.);
        let bounds = NativeRect {
            x: 5.,
            y: 7.,
            width: 1000.,
            height: 600.,
        };
        let panel = rect(117., 123., 1120., 760.);
        assert_eq!(
            native_rect(viewport, bounds, panel, true),
            Some(NativeRect {
                x: 55.,
                y: 57.,
                width: 560.,
                height: 380.
            })
        );
        assert_eq!(
            native_rect(viewport, bounds, panel, false),
            Some(NativeRect {
                x: 55.,
                y: 177.,
                width: 560.,
                height: 380.
            })
        );
        assert_eq!(native_rect(viewport, bounds, viewport, false), Some(bounds));
        assert_eq!(native_rect(viewport, bounds, viewport, true), Some(bounds));
        assert!(native_rect(rect(0., 0., 0., 0.), bounds, panel, false).is_none());
        assert!(
            native_rect(
                viewport,
                NativeRect {
                    width: f64::NAN,
                    ..bounds
                },
                panel,
                false
            )
            .is_none()
        );
        assert!(native_rect(viewport, bounds, rect(f32::NAN, 0., 100., 100.), false).is_none());
    }
    #[test]
    fn deferred_menus_share_their_window_controller_without_leaking_other_windows() {
        let first = dunlin_ui::WindowId::from(0x1_0000_0001);
        let second = dunlin_ui::WindowId::from(0x1_0000_0002);
        let mut registry = super::WindowBackdrops::default();
        let owner = registry.controller(first);
        let menu = registry.controller(first);
        let other = registry.controller(second);
        assert!(std::rc::Rc::ptr_eq(&owner.shared, &menu.shared));
        assert!(!std::rc::Rc::ptr_eq(&owner.shared, &other.shared));
        owner.shared.frame.borrow_mut().surface = Some(rect(10., 20., 100., 200.));
        assert_eq!(
            menu.shared.frame.borrow().foreground(),
            owner.shared.frame.borrow().foreground()
        );
        drop(owner);
        drop(menu);
        registry.controller(second);
        assert_eq!(registry.0.len(), 1);
    }
    #[test]
    fn measured_dialogs_and_anchored_popups_leave_only_the_foreground_sharp() {
        for viewport in [
            rect(0., 0., 1360., 860.),
            rect(0., 0., 1100., 720.),
            rect(17., 23., 1200., 900.),
        ] {
            for panel in [
                rect(244., 129., 872., 602.),
                rect(415., 200., 530., 460.),
                rect(11., 519., 300., 170.),
                rect(900., 42., 190., 220.),
                rect(-40., -30., 300., 160.),
            ] {
                let foreground = panel.intersect(&viewport);
                let regions = regions(viewport, Some(panel), false);
                assert!(regions.len() <= 4);
                assert_eq!(
                    regions.iter().copied().map(area).sum::<f32>() + area(foreground),
                    area(viewport)
                );
                for (index, region) in regions.iter().enumerate() {
                    assert_eq!(area(region.intersect(&foreground)), 0.);
                    assert_eq!(*region, region.intersect(&viewport));
                    for other in &regions[index + 1..] {
                        assert_eq!(area(region.intersect(other)), 0.);
                    }
                }
            }
        }
    }
    #[test]
    fn closing_and_invalid_layouts_do_not_leave_an_orphan_backdrop() {
        let viewport = rect(0., 0., 1360., 860.);
        assert!(regions(viewport, None, false).is_empty());
        assert!(regions(viewport, Some(rect(f32::NAN, 0., 100., 100.)), false).is_empty());
        assert!(regions(viewport, Some(rect(0., 0., 0., 0.)), false).is_empty());
        assert!(regions(viewport, Some(rect(2000., 2000., 100., 100.)), false).is_empty());
        assert!(regions(rect(0., 0., 0., 0.), None, true).is_empty());
        assert!(regions(viewport, Some(viewport), false).is_empty());
    }
    #[test]
    fn scrolling_contents_do_not_move_the_sharp_panel_cutout() {
        let viewport = rect(0., 0., 1360., 860.);
        let panel = rect(450., 100., 460., 650.);
        let before = super::surface_bounds(panel, panel, true);
        let after = super::surface_bounds(rect(450., -200., 460., 650.), panel, true);
        assert_eq!(before, after);
        assert_eq!(
            regions(viewport, Some(before), false),
            regions(viewport, Some(after), false)
        );
        let resized = rect(400., 60., 460., 600.);
        assert_eq!(
            super::surface_bounds(panel, resized, true),
            resized.dilate(px(1.))
        );
        assert_eq!(
            super::surface_bounds(panel, viewport, false),
            panel.dilate(px(1.))
        );
    }
    #[test]
    fn the_topmost_surface_wins_and_menus_preserve_their_submenu_chain() {
        let settings = rect(244., 129., 872., 602.);
        let confirmation = rect(460., 300., 400., 200.);
        let menu = rect(700., 260., 240., 200.);
        let submenu = rect(935., 290., 220., 160.);
        let mut frame = Frame {
            surface: Some(settings),
            menu: None,
            radius: 0.,
        };
        frame.surface = Some(confirmation);
        assert_eq!(frame.foreground(), Some(confirmation));
        frame.menu = Some(menu.union(&submenu));
        assert_eq!(frame.foreground(), Some(menu.union(&submenu)));
        // One region set, not successively compounded overlays for each panel.
        assert!(regions(rect(0., 0., 1360., 860.), frame.foreground(), false).len() <= 4);
        frame = Frame::default();
        assert!(frame.foreground().is_none());
    }
    #[test]
    fn native_prompts_blur_the_entire_owner_without_a_guessed_in_window_hole() {
        let viewport = rect(0., 0., 1360., 860.);
        assert_eq!(regions(viewport, None, true), [viewport]);
        assert_eq!(
            regions(viewport, Some(rect(244., 129., 872., 602.)), true),
            [viewport]
        );
    }
    #[test]
    fn nested_native_leases_release_on_cancel_error_and_unpolled_task_drop() {
        let depth = NativeDepth::default();
        let first = depth.acquire();
        let second = depth.acquire();
        assert_eq!(depth.get(), 2);
        drop(first);
        assert_eq!(depth.get(), 1);
        drop(second);
        assert_eq!(depth.get(), 0);
        let lease = depth.acquire();
        let pending = async move {
            let _lease = lease;
            std::future::pending::<()>().await;
        };
        assert_eq!(depth.get(), 1);
        drop(pending);
        assert_eq!(depth.get(), 0);
        let _: Result<(), &str> = {
            let _lease = depth.acquire();
            Err("cancelled")
        };
        assert_eq!(depth.get(), 0);
    }
    #[test]
    fn both_themes_use_light_dimming_without_mixing_sharp_content_back_into_glass() {
        let light = super::dim_color(&dunlin_ui::theme::light());
        let dark = super::dim_color(&dunlin_ui::theme::dark());
        assert_eq!(light, dark);
        assert!((0.1..=0.15).contains(&light.a));
        assert_eq!(super::GLASS_ALPHA, 1.);
    }
}
#[cfg(not(target_os = "macos"))]
mod native {
    use super::*;
    #[derive(Default)]
    pub(super) struct BackdropViews;
    impl BackdropViews {
        #[cfg(debug_assertions)]
        pub(super) fn panel_ready(&self) -> bool {
            false
        }
        pub(super) fn update_panel(
            &mut self,
            _: &Window,
            _: Bounds<Pixels>,
            _: Option<Bounds<Pixels>>,
            _: f32,
            _: bool,
        ) {
        }
        pub(super) fn update(
            &mut self,
            _: &Window,
            _: Bounds<Pixels>,
            _: &[Bounds<Pixels>],
            _: bool,
            _: Option<(Bounds<Pixels>, f32)>,
        ) {
        }
    }
}
