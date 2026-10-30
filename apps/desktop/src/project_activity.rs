//! Read-only upstream observations, scoped to the visible Dashboard tiles.
use crate::app::DunlinApp;
use chrono::{Local, TimeZone};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Icon;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::{Context, *};
use dunlin_workspace::git::{BranchActivity, RemoteState, RemoteStatus};
use std::time::Instant;

const CACHE_SECONDS: u64 = 120;
pub(crate) struct Check {
    id: uuid::Uuid,
    path: String,
    target: BranchActivity,
    started: Instant,
    checked_ms: Option<i64>,
    result: Option<Result<RemoteStatus, String>>,
}
impl Check {
    fn matches(&self, path: &str, target: &BranchActivity) -> bool {
        self.path == path && self.target.same_target(target)
    }
}
impl DunlinApp {
    pub(crate) fn check_project_remote(&mut self, id: &str, force: bool, cx: &mut Context<Self>) {
        let Some(project) = self.projects.iter().find(|p| p.id == id) else {
            return;
        };
        if !project.trusted || project.is_remote() || self.branch_ui.locks(id) {
            return;
        }
        let Some(summary) = self
            .repo_summaries
            .values
            .get(id)
            .filter(|s| s.path == project.path)
        else {
            return;
        };
        let target = summary.activity.clone();
        if target.local_oid.is_none() || target.remote_name.is_none() || target.remote_ref.is_none()
        {
            return;
        }
        if self
            .repo_summaries
            .remote_checks
            .get(id)
            .is_some_and(|check| {
                check.matches(&project.path, &target)
                    && (check.result.is_none()
                        || (!force && check.started.elapsed().as_secs() < CACHE_SECONDS))
            })
        {
            return;
        }
        // Old requests retain their ownership token until they finish, even
        // when a branch/URL changes. No unbounded overlapping process queue.
        if self
            .repo_summaries
            .remote_checks
            .get(id)
            .is_some_and(|check| check.result.is_none())
            || self
                .repo_summaries
                .remote_checks
                .values()
                .filter(|c| c.result.is_none())
                .count()
                >= 3
        {
            return;
        }
        let request = uuid::Uuid::now_v7();
        let id = id.to_string();
        let path = project.path.clone();
        self.repo_summaries.remote_checks.insert(
            id.clone(),
            Check {
                id: request,
                path: path.clone(),
                target: target.clone(),
                started: Instant::now(),
                checked_ms: None,
                result: None,
            },
        );
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { dunlin_workspace::git::check_remote_status(&path, &target) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(check) = this.repo_summaries.remote_checks.get_mut(&id)
                    && check.id == request
                {
                    check.checked_ms = Some(chrono::Utc::now().timestamp_millis());
                    check.result = Some(result);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn counts_label(ahead: u64, behind: u64) -> String {
    match (ahead, behind) {
        (0, 0) => "Up to date".into(),
        (0, behind) => format!("Pull needed · {behind} behind"),
        (ahead, 0) => format!("{ahead} ahead · no pull needed"),
        (ahead, behind) => format!("Diverged · {behind} behind, {ahead} ahead"),
    }
}
fn status_label(target: &BranchActivity, check: Option<&Check>) -> (String, bool) {
    if target.local_oid.is_none() {
        return ("Git status unavailable".into(), false);
    }
    if target.upstream.is_none() {
        return ("No upstream configured".into(), false);
    }
    if target.remote_name.is_none() {
        return ("Local upstream · no remote check".into(), false);
    }
    if let Some(check) = check {
        let stale = check.started.elapsed().as_secs() >= CACHE_SECONDS;
        let (label, warning) = match &check.result {
            None => return ("Checking remote…".into(), false),
            Some(Err(_)) => return ("Remote check failed · retry".into(), true),
            Some(Ok(status)) => match status.state {
                RemoteState::Changed => ("Remote differs · fetch needed to compare".into(), true),
                RemoteState::Missing => ("Remote branch no longer exists".into(), true),
                RemoteState::Known => match (status.ahead, status.behind) {
                    (Some(ahead), Some(behind)) => (counts_label(ahead, behind), behind > 0),
                    _ => ("Remote observed · comparison unavailable".into(), false),
                },
            },
        };
        return (
            if stale {
                format!("Last check: {label}")
            } else {
                label
            },
            warning,
        );
    }
    let label = match (target.ahead, target.behind) {
        (Some(ahead), Some(behind)) => format!("Cached: {}", counts_label(ahead, behind)),
        _ => "Remote not checked".into(),
    };
    (label, false)
}

pub(crate) fn render(
    this: &DunlinApp,
    project: &dunlin_domain::Project,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let target = this
        .repo_summaries
        .values
        .get(&project.id)
        .filter(|s| s.path == project.path)
        .map(|s| &s.activity);
    let commit = target
        .and_then(|t| t.last_commit_ms)
        .and_then(|ms| i64::try_from(ms).ok())
        .and_then(|ms| Local.timestamp_millis_opt(ms).single())
        .map(|date| format!("Last commit · {}", date.format("%Y-%m-%d %H:%M %Z")))
        .unwrap_or_else(|| "Last commit · unavailable".into());
    let check = target.and_then(|target| {
        this.repo_summaries
            .remote_checks
            .get(&project.id)
            .filter(|check| check.matches(&project.path, target))
    });
    let (label, warning) = if project.is_remote() {
        ("Remote workspace · check locally".into(), false)
    } else if !project.trusted {
        ("Trust project to check remote".into(), false)
    } else {
        target
            .map(|t| status_label(t, check))
            .unwrap_or(("Loading Git information…".into(), false))
    };
    let can_check = project.trusted
        && !project.is_remote()
        && target.is_some_and(|t| t.remote_name.is_some() && t.remote_ref.is_some())
        && check.is_none_or(|c| c.result.is_some())
        && !this.branch_ui.locks(&project.id);
    let id = project.id.clone();
    div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .text_size(px(10.5))
        .text_color(p.muted_fg)
        .child(commit)
        .child(
            div()
                .text_color(if warning { p.amber } else { p.muted_fg })
                .child(label),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div().flex_1().child(
                        check
                            .and_then(|c| c.checked_ms)
                            .and_then(|ms| Local.timestamp_millis_opt(ms).single())
                            .map(|date| format!("{} {}", if check.is_some_and(|c| matches!(c.result, Some(Err(_)))) { "Last attempt" } else { "Checked" }, date.format("%H:%M:%S")))
                            .unwrap_or_else(|| "Tracking data may be stale".into()),
                    ),
                )
                .when(can_check, |row| {
                    row.child(
                        div()
                            .id(format!("check-remote:{id}"))
                            .aria_label("Refresh read-only remote status (no fetch or pull)")
                            .px(px(5.))
                            .py(px(3.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(move |s| s.bg(p.muted))
                            .tooltip(|window, cx| dunlin_ui::component::tooltip::Tooltip::new("Check the advertised upstream commit without fetching or pulling. Cached tracking counts may be stale; unknown remote commits require an explicit fetch to compare.").build(window, cx))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.check_project_remote(&id, true, cx);
                            }))
                            .child(Icon::new(IconName::RefreshCw).size(px(12.))),
                    )
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{Check, counts_label, status_label};
    use dunlin_workspace::git::{BranchActivity, RemoteState, RemoteStatus};
    #[test]
    fn cached_status_never_claims_a_fresh_remote_check() {
        let target = dunlin_workspace::git::BranchActivity {
            local_oid: Some("oid".into()),
            upstream: Some("origin/main".into()),
            remote_name: Some("origin".into()),
            ahead: Some(0),
            behind: Some(0),
            ..Default::default()
        };
        assert_eq!(status_label(&target, None).0, "Cached: Up to date");
        assert!(counts_label(0, 2).contains("Pull needed"));
        assert!(counts_label(1, 2).contains("Diverged"));
    }

    #[test]
    fn observations_are_scoped_to_root_branch_commit_upstream_and_remote_configuration() {
        let target = BranchActivity {
            local_oid: Some("one".into()),
            head_ref: Some("refs/heads/main".into()),
            upstream: Some("origin/main".into()),
            remote_name: Some("origin".into()),
            remote_ref: Some("refs/heads/main".into()),
            canonical_root: Some("/project".into()),
            remote_fingerprint: Some(1),
            ..Default::default()
        };
        let check = Check {
            id: uuid::Uuid::now_v7(),
            path: "/alias".into(),
            target: target.clone(),
            started: std::time::Instant::now(),
            checked_ms: None,
            result: None,
        };
        assert!(check.matches("/alias", &target));
        assert!(!check.matches("/elsewhere", &target));
        let mut changed = target.clone();
        changed.head_ref = Some("refs/heads/other".into());
        assert!(!check.matches("/alias", &changed));
        let mut changed = target.clone();
        changed.local_oid = Some("two".into());
        assert!(!check.matches("/alias", &changed));
        let mut changed = target.clone();
        changed.canonical_root = Some("/retargeted".into());
        assert!(!check.matches("/alias", &changed));
        let mut changed = target.clone();
        changed.remote_ref = Some("refs/heads/other".into());
        assert!(!check.matches("/alias", &changed));
        let mut changed = target;
        changed.remote_fingerprint = Some(2);
        assert!(!check.matches("/alias", &changed));
    }
    #[test]
    fn checking_failure_missing_unknown_and_stale_results_are_not_successful_sync_claims() {
        let target = BranchActivity {
            local_oid: Some("oid".into()),
            upstream: Some("origin/main".into()),
            remote_name: Some("origin".into()),
            ..Default::default()
        };
        let mut check = Check {
            id: uuid::Uuid::now_v7(),
            path: "/project".into(),
            target: target.clone(),
            started: std::time::Instant::now(),
            checked_ms: None,
            result: None,
        };
        assert_eq!(status_label(&target, Some(&check)).0, "Checking remote…");
        check.result = Some(Err("connection failed".into()));
        assert!(status_label(&target, Some(&check)).0.contains("failed"));
        check.result = Some(Ok(RemoteStatus {
            state: RemoteState::Missing,
            ahead: None,
            behind: None,
        }));
        assert!(
            status_label(&target, Some(&check))
                .0
                .contains("no longer exists")
        );
        check.result = Some(Ok(RemoteStatus {
            state: RemoteState::Changed,
            ahead: None,
            behind: None,
        }));
        let label = status_label(&target, Some(&check)).0;
        assert!(label.contains("fetch needed"));
        assert!(!label.contains("behind"));
        check.result = Some(Ok(RemoteStatus {
            state: RemoteState::Known,
            ahead: None,
            behind: None,
        }));
        assert!(!status_label(&target, Some(&check)).0.contains("Up to date"));
        check.result = Some(Ok(RemoteStatus {
            state: RemoteState::Known,
            ahead: Some(0),
            behind: Some(0),
        }));
        check.started =
            std::time::Instant::now() - std::time::Duration::from_secs(super::CACHE_SECONDS + 1);
        assert_eq!(
            status_label(&target, Some(&check)).0,
            "Last check: Up to date"
        );
    }
}
