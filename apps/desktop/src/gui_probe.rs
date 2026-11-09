//! Optional measurement of complete hit targets. No extra layout in normal use.
use dunlin_ui::*;

#[cfg(debug_assertions)]
use std::{
    cell::RefCell,
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};
#[cfg(debug_assertions)]
static ENABLED: AtomicBool = AtomicBool::new(false);
#[cfg(debug_assertions)]
thread_local! {
    static TARGETS: RefCell<BTreeMap<String, Bounds<Pixels>>> = const { RefCell::new(BTreeMap::new()) };
}

pub(crate) fn target(id: impl Into<String>, element: impl IntoElement) -> AnyElement {
    #[cfg(debug_assertions)]
    if ENABLED.load(Ordering::Relaxed) {
        let id = id.into();
        return div()
            .relative()
            .flex()
            .flex_shrink_0()
            .child(element)
            .child(
                canvas(
                    move |bounds, _, _| {
                        TARGETS.with(|targets| {
                            targets.borrow_mut().insert(id.clone(), bounds);
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .into_any_element();
    }
    let _ = id;
    element.into_any_element()
}

#[cfg(debug_assertions)]
pub(crate) fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Mark an existing positioned surface without changing its layout.
pub(crate) fn marker(id: &'static str) -> Option<AnyElement> {
    marker_named(id)
}

pub(crate) fn marker_named(id: impl Into<String>) -> Option<AnyElement> {
    let id = id.into();
    #[cfg(debug_assertions)]
    if ENABLED.load(Ordering::Relaxed) {
        return Some(
            canvas(
                move |bounds, _, _| {
                    TARGETS.with(|targets| {
                        targets.borrow_mut().insert(id.clone(), bounds);
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .into_any_element(),
        );
    }
    let _ = id;
    None
}
#[cfg(debug_assertions)]
pub(crate) fn clear() {
    TARGETS.with(|targets| targets.borrow_mut().clear());
}
#[cfg(debug_assertions)]
pub(crate) fn bounds(id: &str) -> Option<Bounds<Pixels>> {
    TARGETS.with(|targets| targets.borrow().get(id).copied())
}
#[cfg(debug_assertions)]
pub(crate) fn snapshot() -> serde_json::Value {
    TARGETS.with(|targets| serde_json::json!(targets.borrow().iter().map(|(id, b)| {
        serde_json::json!({"id":id,"x":f32::from(b.left()),"y":f32::from(b.top()),"width":f32::from(b.size.width),"height":f32::from(b.size.height)})
    }).collect::<Vec<_>>()))
}
