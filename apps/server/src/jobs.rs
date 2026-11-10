//! Host-owned serial execution. Interrupted paid turns are never replayed.
use crate::{
    api::{ApiError, AppState, AuthUser},
    store::jobs::{Record, Task},
};
use axum::{
    Json,
    extract::{Path, State},
    http::header::{CACHE_CONTROL, HeaderValue},
    response::{IntoResponse, Response},
};
use dunlin_domain::jobs::{Action, Enqueue, Job, Status};
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;
mod runner;
#[cfg(test)]
mod tests;
pub(crate) struct Service {
    enqueue_slots: Arc<Semaphore>,
    #[cfg(test)]
    pub command: Option<(String, Vec<String>)>,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            enqueue_slots: Arc::new(Semaphore::new(2)),
            #[cfg(test)]
            command: None,
        }
    }
}
fn response(value: impl serde::Serialize) -> Response {
    let mut response = Json(value).into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(crate) fn settings(
    state: &AppState,
    project: &str,
) -> Result<(String, dunlin_domain::ProjectSettings), String> {
    let raw = state
        .store
        .lock()
        .map_err(|_| "Store lock poisoned")?
        .project_settings(project)
        .map_err(|_| "Could not read project settings")?
        .unwrap_or_else(|| "{}".into());
    let settings = serde_json::from_str(&raw).map_err(|_| "Invalid server project settings")?;
    Ok((crate::auth::hash_token(&raw), settings))
}
pub(crate) fn authorized(state: &AppState, record: &Record) -> Result<(), String> {
    let store = state.store.lock().map_err(|_| "Store lock poisoned")?;
    let user = store
        .active_device_user(&record.task.device)
        .map_err(|_| "Authorization unavailable")?
        .filter(|user| user.id == record.job.owner)
        .ok_or("Authorizing device was revoked; review and requeue from an active device")?;
    if user.role != "admin" && crate::management::host_accounts(&state.config) {
        return Err("Host account policy no longer allows this user".into());
    }
    let project = store
        .project_for_user(&record.job.project_id, &user.id)
        .map_err(|_| "Project authorization unavailable")?
        .ok_or("Project grant was revoked")?;
    let revision = store
        .linear_revision(&user.id)
        .map_err(|_| "Linear configuration unavailable")?
        .ok_or("Linear was disconnected")?;
    if revision != record.task.linear_revision {
        return Err("Linear configuration changed; reapprove the task".into());
    }
    let current = std::fs::canonicalize(&project.path).map_err(|_| "Project root unavailable")?;
    if current != std::path::Path::new(&record.task.root) {
        return Err("Project root changed; reapprove the task".into());
    }
    drop(store);
    let (digest, settings) = settings(state, &record.job.project_id)?;
    if digest != record.task.settings_digest {
        return Err("Project settings changed; reapprove the task".into());
    }
    if settings.permission_mode == Some(dunlin_domain::PermissionMode::ReadOnly)
        && record.job.profile.allow_tools
    {
        return Err(
            "Project read-only permission policy disallows approving client write/tool requests"
                .into(),
        );
    }
    Ok(())
}
pub(crate) async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Response, ApiError> {
    let mut queue = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store lock poisoned"))?
        .jobs()?;
    queue.self_user_id = Some(auth.user.id.clone());
    queue.jobs.retain(|job| {
        state
            .project_for_user(&auth.user.id, &job.project_id)
            .is_ok()
    });
    for job in &mut queue.jobs {
        if job.owner != auth.user.id && auth.user.role != "admin" {
            job.report.clear();
            job.sync_error = None;
        }
    }
    Ok(response(queue))
}
fn accessible(state: &AppState, auth: &AuthUser, id: &str) -> Result<Record, ApiError> {
    let record = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store lock poisoned"))?
        .job(id)?
        .ok_or_else(ApiError::not_found)?;
    state.project_for_user(&auth.user.id, &record.job.project_id)?;
    Ok(record)
}
pub(crate) async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let mut job = accessible(&state, &auth, &id)?.job;
    if job.owner != auth.user.id && auth.user.role != "admin" {
        job.report.clear();
        job.sync_error = None;
    }
    Ok(response(job))
}
pub(crate) async fn enqueue(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(request): Json<Enqueue>,
) -> Result<Response, ApiError> {
    request.profile.validate().map_err(ApiError::bad_request)?;
    if !request.approved
        || request.issue_id.is_empty()
        || request.issue_id.len() > 128
        || request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 128
        || request.expected_issue_digest.len() != 64
    {
        return Err(ApiError::bad_request(
            "An explicit task/profile approval and bounded idempotency key are required",
        ));
    }
    let project = state.project_for_user(&auth.user.id, &request.project_id)?;
    let connection=state.store.lock().map_err(|_|ApiError::internal("Store lock poisoned"))?.linear_connection(&auth.user.id)?
        .filter(|connection|connection.inbox.error.is_none() && connection.inbox.synced_at.is_some_and(|at|crate::store::now_secs()-at<=120))
        .ok_or_else(||ApiError::conflict("Connect/synchronize Linear on the target host first; the issue must be in your fresh tagged inbox"))?;
    let issue = connection
        .inbox
        .issues
        .into_iter()
        .find(|issue| issue.id == request.issue_id)
        .ok_or_else(|| ApiError::conflict("Issue is not in your target host inbox"))?;
    if dunlin_mcp_client::linear::issue_digest(&issue) != request.expected_issue_digest {
        return Err(ApiError::conflict(
            "The issue snapshot differs on the target host. Refresh its inbox and review the task again.",
        ));
    }
    let _slot = state
        .jobs
        .enqueue_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::conflict("Task preparation is busy"))?;
    let root = tokio::fs::canonicalize(&project.path)
        .await
        .map_err(|_| ApiError::bad_request("Project root unavailable"))?
        .to_string_lossy()
        .into_owned();
    let mut command = tokio::process::Command::new("git");
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        command
            .args(["-C", &root, "rev-parse", "--verify", "HEAD"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| ApiError::conflict("Git HEAD lookup timed out"))?
    .map_err(|_| ApiError::bad_request("Git is unavailable"))?;
    let head = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success()
        || !matches!(head.len(), 40 | 64)
        || !head.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ApiError::bad_request("Jobs require an existing Git commit"));
    }
    let task = Task {
        linear_revision: connection.revision,
        settings_digest: settings(&state, &project.id)
            .map_err(ApiError::bad_request)?
            .0,
        request,
        issue,
        root,
        head,
        device: auth.device.id.clone(),
    };
    let provisional = Record {
        job: Job {
            id: String::new(),
            owner: auth.user.id.clone(),
            owner_name: auth.user.name,
            project_id: project.id,
            issue_id: task.issue.id.clone(),
            identifier: task.issue.identifier.clone(),
            title: task.issue.title.clone(),
            profile: task.request.profile.clone(),
            status: Status::Queued,
            cancel_requested: false,
            report: String::new(),
            checkout_project: None,
            branch: None,
            sync_state: "none".into(),
            sync_error: None,
        },
        task,
    };
    authorized(&state, &provisional).map_err(|_| ApiError::forbidden())?;
    let job = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store lock poisoned"))?
        .enqueue(&auth.user.id, &provisional.task)
        .map_err(|error| ApiError::conflict(error.to_string()))?;
    Ok(response(job))
}
pub(crate) async fn action(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(action): Json<Action>,
) -> Result<Response, ApiError> {
    let record = accessible(&state, &auth, &id)?;
    if record.job.owner != auth.user.id {
        crate::management::admin(&auth)?;
    }
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store lock poisoned"))?;
    match action.action.as_str() {
        "cancel" => store.cancel_job(&id)?,
        "front" => {
            crate::management::admin(&auth)?;
            store.prioritize_job(&id)?;
        }
        "retry_status" if action.approved => store.retry_status_delivery(&id)?,
        "resolve_status" if action.approved => store.resolve_status_delivery(&id)?,
        "archive" => store.archive_job(&id)?,
        _ => return Err(ApiError::bad_request("Unknown/unapproved job action")),
    }
    Ok(response(store.job(&id)?.map(|record| record.job)))
}
pub(crate) async fn queue_action(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(action): Json<Action>,
) -> Result<Response, ApiError> {
    crate::management::admin(&auth)?;
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store lock poisoned"))?;
    match action.action.as_str() {
        "pause" => store.pause_jobs(true)?,
        "resume" => store.pause_jobs(false)?,
        "reconcile" if action.approved => store.reconcile_jobs()?,
        _ => return Err(ApiError::bad_request("Unknown/unapproved queue action")),
    }
    Ok(response(serde_json::json!({"ok":true})))
}
pub(crate) fn start_background(state: AppState) -> Result<(), String> {
    state
        .store
        .lock()
        .map_err(|_| "Store lock poisoned")?
        .recover_jobs()
        .map_err(|error| error.to_string())?;
    let worker = state.clone();
    tokio::spawn(async move {
        loop {
            let record = worker
                .store
                .lock()
                .ok()
                .and_then(|store| store.claim_job().ok())
                .flatten();
            if let Some(record) = record {
                let id = record.job.id.clone();
                let (status, report) = match runner::execute(&worker, &record).await {
                    Ok(report) => (Status::Review, report),
                    Err(result) => result,
                };
                if let Ok(store) = worker.store.lock() {
                    let _ = store.finish_job(&id, status, &report);
                }
            } else {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    });
    tokio::spawn(async move {
        loop {
            let record = state
                .store
                .lock()
                .ok()
                .and_then(|store| store.claim_status_delivery().ok())
                .flatten();
            if let Some(record) = record {
                let id = record.job.id.clone();
                let run = state.clone();
                let result =
                    tokio::task::spawn_blocking(move || crate::linear::job_status(&run, &record))
                        .await;
                if let Ok(store) = state.store.lock() {
                    match result {
                        Ok(Ok(())) => {
                            let _ = store.finish_status_delivery(&id, None);
                        }
                        Ok(Err(error)) if error.retry => {
                            let _ = store.defer_status_delivery(&id, &error.message);
                        }
                        Ok(Err(error)) => {
                            let _ = store.finish_status_delivery(&id, Some(&error.message));
                        }
                        Err(_) => {
                            let _ = store.finish_status_delivery(
                                &id,
                                Some("Status worker interrupted; inspect Linear before retrying"),
                            );
                        }
                    }
                }
            } else {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    });
    Ok(())
}
