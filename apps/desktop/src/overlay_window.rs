//! Transparent, owner-attached render plane. Glass is a native sibling BELOW
//! this plane's GPUI controls, not a filter over the application's entire scene.
use crate::{app::DunlinApp, effects::OverlayBackdrop};
use dunlin_ui::*;

#[derive(Default)]
pub(crate) struct State {
    window: Option<WindowHandle<component::Root>>,
    plane: Option<WeakEntity<OverlayWindow>>,
    scheduled: bool,
    failed: bool,
    focus: Option<FocusHandle>,
    pub active: bool,
}
impl State {
    #[cfg(debug_assertions)]
    pub(crate) fn diagnostic(&self, cx: &App) -> Option<Bounds<Pixels>> {
        self.plane
            .as_ref()?
            .upgrade()?
            .read(cx)
            .backdrop
            .panel_diagnostic()
    }
    #[cfg(debug_assertions)]
    pub(crate) fn is_open(&self) -> bool {
        self.window.is_some()
    }
    #[cfg(debug_assertions)]
    pub(crate) fn debug_window(&self) -> Option<AnyWindowHandle> {
        self.window.map(Into::into)
    }
}

pub(crate) fn prepare(app: &mut DunlinApp, window: &Window, cx: &mut Context<DunlinApp>) -> bool {
    let requested = app.has_modal_overlay();
    if !requested {
        app.overlay_window.failed = false;
    }
    let detached = cfg!(target_os = "macos") && requested && !app.overlay_window.failed;
    app.overlay_backdrop.set_external_modal(detached);
    if (detached || app.overlay_window.window.is_some()) && !app.overlay_window.scheduled {
        app.overlay_window.scheduled = true;
        let owner = cx.entity();
        let parent = window.window_handle();
        // open_window draws immediately: run outside an owner entity borrow,
        // otherwise the child renderer would re-enter the borrowed owner.
        cx.defer(move |cx| reconcile(owner, parent, cx));
    }
    detached
}

fn reconcile(owner: Entity<DunlinApp>, parent: AnyWindowHandle, cx: &mut App) {
    let (requested, existing, failed, backdrop, focus) = owner.update(cx, |app, cx| {
        app.overlay_window.scheduled = false;
        (
            app.has_modal_overlay(),
            app.overlay_window.window,
            app.overlay_window.failed,
            app.overlay_backdrop.clone(),
            app.overlay_focus_handle(cx),
        )
    });
    if !requested {
        owner.update(cx, |app, _| {
            app.overlay_window.window = None;
            app.overlay_window.plane = None;
            app.overlay_window.focus = None;
            app.overlay_window.active = false;
            app.overlay_backdrop.set_external_modal(false);
        });
        if let Some(handle) = existing {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        let _ = parent.update(cx, |_, window, cx| {
            window.activate_window();
            focus.focus(window, cx);
        });
        return;
    }
    if failed {
        return;
    }
    if let Some(handle) = existing {
        let focus_changed = owner.read(cx).overlay_window.focus.as_ref() != Some(&focus);
        let result = parent.update(cx, |_, parent_window, cx| {
            handle.update(cx, |_, window, cx| {
                if !native::attach(window, parent_window) {
                    return false;
                }
                // Direct AppKit frame changes run while this GPUI window is
                // borrowed, so its re-entrant platform callback cannot update
                // the cached viewport. Refresh it explicitly after the bridge.
                if window.viewport_size() != parent_window.viewport_size() {
                    window.bounds_changed(cx);
                }
                if focus_changed {
                    window.activate_window();
                    focus.focus(window, cx);
                }
                true
            })
        });
        if !matches!(result, Ok(Ok(true))) {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
            owner.update(cx, |app, cx| {
                app.overlay_window.window = None;
                app.overlay_window.plane = None;
                app.overlay_window.focus = None;
                app.overlay_window.active = false;
                app.overlay_window.failed = true;
                app.overlay_backdrop.set_external_modal(false);
                cx.notify();
            });
        } else {
            owner.update(cx, |app, _| app.overlay_window.focus = Some(focus));
            // Rendering the owner's data through another entity isn't itself
            // a child-view render dependency. Explicitly invalidate that view
            // after owner reconciliation, outside either window's draw phase.
            let plane = owner
                .read(cx)
                .overlay_window
                .plane
                .as_ref()
                .and_then(WeakEntity::upgrade);
            if let Some(plane) = plane {
                plane.update(cx, |_, cx| cx.notify());
            }
        }
        return;
    }
    let Ok(bounds) = parent.update(cx, |_, window, _| window.bounds()) else {
        return;
    };
    let weak = owner.downgrade();
    let mut built = None;
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_background: WindowBackgroundAppearance::Transparent,
            kind: WindowKind::PopUp,
            titlebar: None,
            focus: false,
            show: false,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| OverlayWindow::new(weak, parent, backdrop, window, cx));
            built = Some(view.downgrade());
            cx.new(|cx| {
                let mut root = component::Root::new(view, window, cx);
                // Override the component root's opaque theme background, while
                // retaining its focus traversal, dropdowns and text-selection host.
                root.style().background = Some(rgba(0x00000000).into());
                root
            })
        },
    );
    match result {
        Ok(handle) => {
            let attached = parent.update(cx, |_, parent_window, cx| {
                handle.update(cx, |_, window, cx| {
                    if !native::attach(window, parent_window) {
                        return false;
                    }
                    if window.viewport_size() != parent_window.viewport_size() {
                        window.bounds_changed(cx);
                    }
                    window.activate_window();
                    focus.focus(window, cx);
                    true
                })
            });
            if !matches!(attached, Ok(Ok(true))) {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
            owner.update(cx, |app, cx| {
                app.overlay_window.window = matches!(attached, Ok(Ok(true))).then_some(handle);
                if app.overlay_window.window.is_some() {
                    app.overlay_window.plane = built;
                    app.overlay_window.focus = Some(focus);
                }
                app.overlay_window.failed = app.overlay_window.window.is_none();
                cx.notify();
            });
        }
        Err(error) => owner.update(cx, |app, cx| {
            app.overlay_window.failed = true;
            app.overlay_backdrop.set_external_modal(false);
            crate::logging::error(&format!("could not open glass panel plane: {error}"));
            app.file_notice("Native panel unavailable; using the in-window dialog", cx);
        }),
    }
}

pub(crate) struct OverlayWindow {
    owner: WeakEntity<DunlinApp>,
    backdrop: OverlayBackdrop,
    _changes: Subscription,
    _release: Subscription,
    _activation: Subscription,
    _bounds: Subscription,
}
impl OverlayWindow {
    fn new(
        owner: WeakEntity<DunlinApp>,
        parent: AnyWindowHandle,
        backdrop: OverlayBackdrop,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let strong = owner
            .upgrade()
            .expect("overlay owner exists while opening its window");
        let changes = cx.observe(&strong, |_, _, cx| cx.notify());
        let release = cx.observe_release(&strong, |_, _, cx| cx.notify());
        let active_owner = owner.clone();
        let activation = cx.observe_window_activation(window, move |_, window, cx| {
            let child_active = window.is_window_active();
            let parent_active = parent
                .update(cx, |_, window, _| window.is_window_active())
                .unwrap_or(false);
            let _ = active_owner.update(cx, |app, _| {
                app.overlay_window.active = child_active;
                app.app_active = child_active || parent_active;
            });
        });
        let bounds = cx.observe_window_bounds(window, |_, _, cx| cx.notify());
        // Existing Close/Escape/outside-click handlers own dismissal and safety
        // gates. Do not let an invisible OS close button orphan modal state.
        window.on_window_should_close(cx, |_, _| false);
        Self {
            owner,
            backdrop: crate::effects::for_panel_window(window, cx, &backdrop),
            _changes: changes,
            _release: release,
            _activation: activation,
            _bounds: bounds,
        }
    }
}
impl Render for OverlayWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(owner) = self.owner.upgrade() else {
            window.remove_window();
            return div().into_any_element();
        };
        let backdrop = self.backdrop.clone();
        owner.update(cx, |app, cx| {
            let original = std::mem::replace(&mut app.overlay_backdrop, backdrop);
            let content = app.render_overlay_plane(window, cx);
            app.overlay_backdrop = original;
            content
        })
    }
}

#[cfg(target_os = "macos")]
mod native {
    use dunlin_ui::Window;
    use objc2_app_kit::{NSView, NSWindowOrderingMode, NSWindowStyleMask};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    pub(super) fn attach(child: &Window, parent: &Window) -> bool {
        let (Ok(c), Ok(p)) = (
            HasWindowHandle::window_handle(child),
            HasWindowHandle::window_handle(parent),
        ) else {
            return false;
        };
        let (RawWindowHandle::AppKit(c), RawWindowHandle::AppKit(p)) = (c.as_raw(), p.as_raw())
        else {
            return false;
        };
        // SAFETY: Both handles borrow live NSViews on GPUI's main thread.
        let (cv, pv): (&NSView, &NSView) =
            unsafe { (&*c.ns_view.as_ptr().cast(), &*p.ns_view.as_ptr().cast()) };
        let (Some(child), Some(parent)) = (cv.window(), pv.window()) else {
            return false;
        };
        if child.parentWindow().as_deref() != Some(&*parent) {
            child.setStyleMask(NSWindowStyleMask::Borderless);
            child.setHasShadow(false); // only the localized panels cast shadows
            child.setLevel(parent.level());
            // SAFETY: A fresh separate child, never its own parent. AppKit owns
            // the relationship and keeps movement/Spaces tied to the owner.
            unsafe {
                parent.addChildWindow_ordered(&child, NSWindowOrderingMode::Above);
            }
        }
        if child.frame() != parent.frame() {
            child.setFrame_display(parent.frame(), true);
        }
        true
    }
}
#[cfg(not(target_os = "macos"))]
mod native {
    pub(super) fn attach(_: &dunlin_ui::Window, _: &dunlin_ui::Window) -> bool {
        false
    }
}
