//! Small macOS integrations that do not belong to a single view.

use dunlin_ui::*;

/// Show a native notification through GPUI's `UNUserNotificationCenter`
/// integration. Authorization is requested lazily on the first notification;
/// development binaries without an app bundle are ignored by the platform.
pub(crate) fn notify(cx: &App, title: &str, body: &str) {
    cx.show_system_notification(SystemNotification {
        tag: format!("dunlin-{}", uuid::Uuid::now_v7()).into(),
        title: title.to_string().into(),
        body: body.to_string().into(),
        actions: Vec::new(),
    });
}

/// Set or clear the Dock badge. Must be called on the main thread.
pub(crate) fn set_dock_badge(label: Option<&str>) {
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        use objc2_foundation::NSString;

        let Some(marker) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(marker);
        let tile = app.dockTile();
        match label {
            Some(label) => tile.setBadgeLabel(Some(&NSString::from_str(label))),
            None => tile.setBadgeLabel(None),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = label;
    }
}
