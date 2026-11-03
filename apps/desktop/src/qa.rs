//! Copyable release QA evidence. Automatic facts only; manual TCC/GUI/real-host
//! acceptance is never inferred from this report.
use crate::app::DunlinApp;
use crate::platform::PermissionStatus;

#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub version: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    pub data_dir: String,
    pub data_writable: bool,
    pub schema_version: Option<i64>,
    pub quick_check: String,
    pub storage_bytes: u64,
    pub projects: usize,
    pub local_projects: usize,
    pub remote_projects: usize,
    pub trusted_projects: usize,
    pub sessions: usize,
    pub current_sessions: usize,
    pub largest_project_bytes: Option<u64>,
    pub ready_harnesses: Vec<&'static str>,
    pub missing_harnesses: Vec<&'static str>,
    pub permissions: Vec<PermissionStatus>,
    pub remote_hosts: usize,
    pub onboarding_step: u8,
    pub onboarding_visible: bool,
}

pub(crate) fn capture(app: &DunlinApp) -> String {
    let data_dir = crate::app::data_dir();
    let data_writable = data_dir.as_ref().is_some_and(|directory| {
        let probe = directory.join(format!(".qa-probe-{}", uuid::Uuid::now_v7()));
        let writable = std::fs::write(&probe, b"qa").is_ok();
        let _ = std::fs::remove_file(&probe);
        writable
    });
    let (schema_version, quick_check) = app
        .store
        .as_ref()
        .map(|store| {
            (
                store.schema_version().ok(),
                store
                    .quick_check()
                    .unwrap_or_else(|error| format!("error: {error}")),
            )
        })
        .unwrap_or((None, "storage unavailable".into()));
    let ready_harnesses = app
        .harness_discovery
        .iter()
        .filter(|detected| {
            crate::harness::command_available(&crate::harness::harness_command(detected.name, ""))
        })
        .map(|detected| detected.name)
        .collect::<Vec<_>>();
    let missing_harnesses = app
        .harness_discovery
        .iter()
        .filter(|detected| {
            !crate::harness::command_available(&crate::harness::harness_command(detected.name, ""))
        })
        .map(|detected| detected.name)
        .collect::<Vec<_>>();
    render(&Snapshot {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        data_dir: crate::app::data_dir_display(),
        data_writable,
        schema_version,
        quick_check,
        storage_bytes: crate::app::storage_size(),
        projects: app.projects.len(),
        local_projects: app
            .projects
            .iter()
            .filter(|project| !project.is_remote())
            .count(),
        remote_projects: app
            .projects
            .iter()
            .filter(|project| project.is_remote())
            .count(),
        trusted_projects: app
            .projects
            .iter()
            .filter(|project| project.trusted)
            .count(),
        sessions: app.all_sessions.len(),
        current_sessions: app.sessions.len(),
        largest_project_bytes: app
            .repo_summaries
            .values
            .values()
            .filter_map(|summary| summary.size_bytes)
            .max(),
        ready_harnesses,
        missing_harnesses,
        permissions: crate::platform::permission_statuses(),
        remote_hosts: app.remote_hosts.len(),
        onboarding_step: app.onboarding_step,
        onboarding_visible: app.projects.is_empty(),
    })
}

pub(crate) fn render(snapshot: &Snapshot) -> String {
    let mut lines = vec![
        "# Dunlin QA report".to_string(),
        format!(
            "Generated: {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z")
        ),
        format!(
            "Version: {} · {} {}",
            snapshot.version, snapshot.os, snapshot.arch
        ),
        format!(
            "Data directory: {} (writable: {})",
            snapshot.data_dir,
            if snapshot.data_writable { "yes" } else { "no" }
        ),
        format!(
            "Database: schema {} · quick_check {} · {}",
            snapshot
                .schema_version
                .map(|version| version.to_string())
                .unwrap_or_else(|| "unknown".into()),
            snapshot.quick_check,
            crate::app::format_bytes(snapshot.storage_bytes as usize)
        ),
        format!(
            "Projects: {} total ({} local, {} remote, {} trusted)",
            snapshot.projects,
            snapshot.local_projects,
            snapshot.remote_projects,
            snapshot.trusted_projects
        ),
        format!(
            "Sessions: {} total · {} in the current project",
            snapshot.sessions, snapshot.current_sessions
        ),
        format!(
            "Largest indexed project: {}",
            snapshot
                .largest_project_bytes
                .map(crate::app::format_size)
                .unwrap_or_else(|| "not indexed".into())
        ),
        format!(
            "Harnesses ready: {}",
            if snapshot.ready_harnesses.is_empty() {
                "none".into()
            } else {
                snapshot.ready_harnesses.join(", ")
            }
        ),
        format!(
            "Harnesses unavailable: {}",
            if snapshot.missing_harnesses.is_empty() {
                "none".into()
            } else {
                snapshot.missing_harnesses.join(", ")
            }
        ),
        format!("Remote hosts: {}", snapshot.remote_hosts),
        format!(
            "Onboarding: {} · step {}",
            if snapshot.onboarding_visible {
                "visible (no projects)"
            } else {
                "complete"
            },
            snapshot.onboarding_step
        ),
        String::new(),
        "## macOS permissions".into(),
    ];
    for status in &snapshot.permissions {
        lines.push(format!(
            "- {}: {} — {}",
            status.kind.label(),
            status.state.label(),
            status.detail
        ));
    }
    let attention = snapshot
        .permissions
        .iter()
        .filter(|status| status.state.needs_attention())
        .count();
    lines.push(String::new());
    lines.push(format!("Permissions needing attention: {attention}"));
    lines.push(String::new());
    lines.push("Manual R11 status: NOT evaluated by this report.".into());
    lines.push(
        "Fresh macOS account, TCC prompts/denial, GUI/IME/bidi/a11y, DB upgrade, large project and real relay/GitHub acceptance still require the manual checklist."
            .into(),
    );
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{Snapshot, render};
    use crate::platform::{PermissionKind, PermissionState, PermissionStatus};

    fn snapshot() -> Snapshot {
        Snapshot {
            version: "0.1.0-test",
            os: "macos",
            arch: "aarch64",
            data_dir: "~/Library/Application Support/Dunlin".into(),
            data_writable: true,
            schema_version: Some(7),
            quick_check: "ok".into(),
            storage_bytes: 42,
            projects: 1,
            local_projects: 1,
            remote_projects: 0,
            trusted_projects: 1,
            sessions: 3,
            current_sessions: 2,
            largest_project_bytes: Some(1024),
            ready_harnesses: vec!["OpenCode"],
            missing_harnesses: vec!["Claude Code"],
            permissions: vec![PermissionStatus {
                kind: PermissionKind::Notifications,
                state: PermissionState::Denied,
                detail: "Notifications are denied in System Settings.".into(),
            }],
            remote_hosts: 0,
            onboarding_step: 1,
            onboarding_visible: false,
        }
    }

    #[test]
    fn report_is_copyable_and_never_claims_manual_acceptance() {
        let report = render(&snapshot());
        assert!(report.contains("# Dunlin QA report"));
        assert!(report.contains("schema 7 · quick_check ok"));
        assert!(report.contains("Projects: 1 total (1 local, 0 remote, 1 trusted)"));
        assert!(report.contains("Harnesses ready: OpenCode"));
        assert!(report.contains("Notifications: Denied"));
        assert!(report.contains("Permissions needing attention: 1"));
        assert!(report.contains("Manual R11 status: NOT evaluated by this report."));
        assert!(!report.contains("manual R11 status: passed"));
    }
}
