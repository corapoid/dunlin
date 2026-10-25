//! Small macOS integrations that do not belong to a single view.

/// Show a native notification. Failures are logged, never fatal: a missing
/// notification permission must not interrupt an agent turn.
pub(crate) fn notify(title: &str, body: &str) {
    // `mac-notification-sys` asks LaunchServices for the bundle identifier. A
    // bare `target/debug/kestrel` has none, and macOS reacts by opening a
    // "Choose Application" panel. Skip notifications outside a real app bundle;
    // the release app always runs from `Kestrel.app/Contents/MacOS`.
    let in_bundle = std::env::current_exe()
        .ok()
        .and_then(|path| path.to_str().map(str::to_string))
        .is_some_and(|path| path.contains(".app/Contents/MacOS/"));
    if !in_bundle {
        crate::logging::info("notification skipped: not running from an app bundle");
        return;
    }
    let mut notification = notify_rust::Notification::new();
    notification.appname("Kestrel").summary(title).body(body);
    if let Err(error) = notification.show() {
        crate::logging::warn(&format!("notification: {error}"));
    }
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
