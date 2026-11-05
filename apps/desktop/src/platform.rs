//! Small macOS integrations that do not belong to a single view.

use dunlin_ui::*;

/// Query on render so changes to the system accessibility setting take effect
/// without relaunching. AppKit owns the underlying native window material.
pub(crate) fn reduce_transparency() -> bool {
    #[cfg(target_os = "macos")]
    {
        objc2_app_kit::NSWorkspace::sharedWorkspace()
            .accessibilityDisplayShouldReduceTransparency()
    }
    #[cfg(not(target_os = "macos"))]
    { true }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PermissionKind {
    Notifications,
    Accessibility,
    ScreenRecording,
    Automation,
    FullDiskAccess,
}

impl PermissionKind {
    pub const ALL: [Self; 5] = [
        Self::Notifications,
        Self::Accessibility,
        Self::ScreenRecording,
        Self::Automation,
        Self::FullDiskAccess,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Notifications => "Notifications",
            Self::Accessibility => "Accessibility",
            Self::ScreenRecording => "Screen Recording",
            Self::Automation => "Automation",
            Self::FullDiskAccess => "Full Disk Access",
        }
    }
    pub fn settings_url(self) -> &'static str {
        match self {
            Self::Notifications => {
                "x-apple.systempreferences:com.apple.Notifications-Settings.extension"
            }
            Self::Accessibility => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
            Self::ScreenRecording => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
            }
            Self::Automation => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation"
            }
            Self::FullDiskAccess => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PermissionState {
    Granted,
    Denied,
    NotDetermined,
    NotGranted,
    Unknown,
}

impl PermissionState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Granted => "Granted",
            Self::Denied => "Denied",
            Self::NotDetermined => "Not determined",
            Self::NotGranted => "Not granted",
            Self::Unknown => "Unknown",
        }
    }

    pub fn needs_attention(self) -> bool {
        matches!(self, Self::Denied | Self::NotDetermined | Self::NotGranted)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PermissionStatus {
    pub kind: PermissionKind,
    pub state: PermissionState,
    pub detail: String,
}

type PermissionCache = std::sync::Mutex<Option<(std::time::Instant, Vec<PermissionStatus>)>>;

static PERMISSION_CACHE: std::sync::OnceLock<PermissionCache> = std::sync::OnceLock::new();

fn permission_cache() -> &'static PermissionCache {
    PERMISSION_CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

pub(crate) fn permission_statuses() -> Vec<PermissionStatus> {
    if let Ok(cache) = permission_cache().lock()
        && let Some((at, statuses)) = cache.as_ref()
        && at.elapsed() < std::time::Duration::from_secs(30)
    {
        return statuses.clone();
    }
    refresh_permission_statuses()
}

pub(crate) fn refresh_permission_statuses() -> Vec<PermissionStatus> {
    let statuses = PermissionKind::ALL
        .into_iter()
        .map(|kind| {
            let (state, detail) = permission_status(kind);
            PermissionStatus {
                kind,
                state,
                detail,
            }
        })
        .collect::<Vec<_>>();
    if let Ok(mut cache) = permission_cache().lock() {
        *cache = Some((std::time::Instant::now(), statuses.clone()));
    }
    statuses
}

pub(crate) fn open_permission_settings(kind: PermissionKind) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("/usr/bin/open")
            .arg(kind.settings_url())
            .status()
            .map_err(|error| format!("Could not open System Settings: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("System Settings could not be opened".into())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = kind;
        Err("macOS permission settings are unavailable on this platform".into())
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
}

#[cfg(target_os = "macos")]
fn notification_status() -> (PermissionState, String) {
    if !notification_bundle_available() {
        return (
            PermissionState::Unknown,
            "Notification status is unavailable outside a signed app bundle; no prompt was requested.".into(),
        );
    }
    use block2::RcBlock;
    use objc2_user_notifications::{
        UNAuthorizationStatus, UNNotificationSettings, UNUserNotificationCenter,
    };
    use std::ptr::NonNull;
    use std::sync::mpsc;

    let center = UNUserNotificationCenter::currentNotificationCenter();
    let (tx, rx) = mpsc::sync_channel(1);
    let block = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        let status = unsafe { settings.as_ref().authorizationStatus() };
        let _ = tx.try_send(status);
    });
    center.getNotificationSettingsWithCompletionHandler(&block);
    match rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(status) if status == UNAuthorizationStatus::Authorized => (
            PermissionState::Granted,
            "Dunlin may post native notifications.".into(),
        ),
        Ok(status) if status == UNAuthorizationStatus::Provisional => (
            PermissionState::Granted,
            "Provisional notification delivery is allowed.".into(),
        ),
        Ok(status) if status == UNAuthorizationStatus::Denied => (
            PermissionState::Denied,
            "Notifications are denied in System Settings.".into(),
        ),
        Ok(status) if status == UNAuthorizationStatus::NotDetermined => (
            PermissionState::NotDetermined,
            "Dunlin requests notification authorization when it first needs to notify.".into(),
        ),
        _ => (
            PermissionState::Unknown,
            "The notification status could not be read; no permission was requested.".into(),
        ),
    }
}

#[cfg(target_os = "macos")]
fn notification_bundle_available() -> bool {
    std::env::current_exe()
        .is_ok_and(|path| path.to_string_lossy().contains(".app/Contents/MacOS/"))
}

pub(crate) fn notification_permission() -> PermissionStatus {
    permission_statuses()
        .into_iter()
        .find(|status| status.kind == PermissionKind::Notifications)
        .unwrap_or(PermissionStatus {
            kind: PermissionKind::Notifications,
            state: PermissionState::Unknown,
            detail: "The notification status could not be read.".into(),
        })
}

/// Request the system notification prompt. Run from a background task because
/// the completion waits for the user's answer in System Settings.
pub(crate) fn request_notification_authorization() -> Result<PermissionState, String> {
    #[cfg(target_os = "macos")]
    {
        if !notification_bundle_available() {
            return Err(
                "Notification authorization requires the packaged Dunlin.app bundle".into(),
            );
        }
        use block2::RcBlock;
        use objc2::runtime::Bool;
        use objc2_foundation::NSError;
        use objc2_user_notifications::{UNAuthorizationOptions, UNUserNotificationCenter};
        use std::sync::mpsc;

        let center = UNUserNotificationCenter::currentNotificationCenter();
        let (tx, rx) = mpsc::sync_channel(1);
        let block = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
            let _ = tx.try_send(granted.as_bool());
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert
                | UNAuthorizationOptions::Sound
                | UNAuthorizationOptions::Badge,
            &block,
        );
        match rx.recv_timeout(std::time::Duration::from_secs(120)) {
            Ok(true) => Ok(PermissionState::Granted),
            Ok(false) => Ok(PermissionState::Denied),
            Err(_) => Err("Notification authorization did not finish".into()),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("macOS notification authorization is unavailable on this platform".into())
    }
}

#[cfg(target_os = "macos")]
fn permission_status(kind: PermissionKind) -> (PermissionState, String) {
    match kind {
        PermissionKind::Notifications => notification_status(),
        PermissionKind::Accessibility => {
            let granted = unsafe { AXIsProcessTrusted() };
            (
                if granted {
                    PermissionState::Granted
                } else {
                    PermissionState::NotGranted
                },
                if granted {
                    "Accessibility is granted to this build.".into()
                } else {
                    "Accessibility is not granted. macOS does not expose denied vs. not-determined without a prompt; Dunlin does not request it for ordinary chat.".into()
                },
            )
        }
        PermissionKind::ScreenRecording => {
            let granted = unsafe { CGPreflightScreenCaptureAccess() };
            (
                if granted {
                    PermissionState::Granted
                } else {
                    PermissionState::NotGranted
                },
                if granted {
                    "Screen Recording is granted; Dunlin still captures nothing unless a feature asks.".into()
                } else {
                    "Screen Recording is not granted. Checking does not capture the screen or trigger a prompt; Dunlin captures nothing by default.".into()
                },
            )
        }
        PermissionKind::Automation => (
            PermissionState::Unknown,
            "Automation grants are per target application and cannot be read without triggering a system prompt. Open System Settings to review them.".into(),
        ),
        PermissionKind::FullDiskAccess => {
            let probe = "/Library/Application Support/com.apple.TCC/TCC.db";
            match std::fs::File::open(probe) {
                Ok(_) => (
                    PermissionState::Granted,
                    "A protected system database is readable, which indicates Full Disk Access.".into(),
                ),
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => (
                    PermissionState::Denied,
                    "Protected system data is not readable. Grant Full Disk Access only if a feature needs it.".into(),
                ),
                Err(_) => (
                    PermissionState::Unknown,
                    "Full Disk Access could not be determined from this build.".into(),
                ),
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn permission_status(kind: PermissionKind) -> (PermissionState, String) {
    let _ = kind;
    (
        PermissionState::Unknown,
        "macOS permission status is unavailable on this platform.".into(),
    )
}

pub(crate) fn trash(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSFileManager, NSString, NSURL};
        let path = path.to_str().ok_or("The path is not valid UTF-8")?;
        NSFileManager::defaultManager()
            .trashItemAtURL_resultingItemURL_error(
                &NSURL::fileURLWithPath(&NSString::from_str(path)),
                None,
            )
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err("System Trash is unavailable on this platform".into())
    }
}

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

#[cfg(test)]
mod tests {
    use super::{PermissionKind, PermissionState};

    #[test]
    fn permission_catalog_has_stable_labels_and_system_settings_links() {
        assert_eq!(PermissionKind::ALL.len(), 5);
        for kind in PermissionKind::ALL {
            assert!(!kind.label().is_empty());
            assert!(
                kind.settings_url()
                    .starts_with("x-apple.systempreferences:")
            );
        }
        assert_eq!(PermissionState::NotGranted.label(), "Not granted");
        assert!(PermissionState::Denied.needs_attention());
        assert!(PermissionState::NotDetermined.needs_attention());
        assert!(PermissionState::NotGranted.needs_attention());
        assert!(!PermissionState::Granted.needs_attention());
        assert!(!PermissionState::Unknown.needs_attention());
    }

    #[test]
    fn permission_center_statuses_are_complete_and_bounded() {
        let statuses = super::permission_statuses();
        assert_eq!(statuses.len(), PermissionKind::ALL.len());
        assert!(statuses.iter().all(|status| !status.detail.is_empty()));
        assert!(
            statuses
                .iter()
                .any(|status| status.kind == PermissionKind::Notifications)
        );
    }
}
