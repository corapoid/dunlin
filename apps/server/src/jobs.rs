//! Host-owned serial execution. Interrupted paid turns are never replayed.
use crate::{
    api::{ApiError, AppState, AuthUser},
    store::jobs::{Record, Task},
    store::{
        local_metadata::MetadataPermission,
        native_checkout::{NativeCheckoutAuthority, NativeCheckoutStamp},
    },
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
    store
        .native_checkout_transaction(|store| {
            authorized_in_store(state, store, record).map_err(crate::store::StoreError::Conflict)
        })
        .map_err(|e| e.to_string())
}
pub(crate) fn authorized_in_store(
    state: &AppState,
    store: &crate::store::Store,
    record: &Record,
) -> Result<(), String> {
    let user = crate::management::authority::current_user_locked(
        state,
        store,
        &record.job.owner,
        &record.task.device,
    )
    .map_err(|_| "Authorizing device, OIDC or host policy changed")?;
    let stamp = record
        .task
        .source_checkout_stamp
        .as_ref()
        .ok_or("Legacy task has no pinned checkout authority; review and enqueue again")?;
    if stamp.native_grant.project_id != record.job.project_id
        || stamp.native_grant.user_id != user.id
        || stamp.permission != MetadataPermission::Read
        || record.task.native_grant.as_ref() != Some(&stamp.native_grant)
        || record.task.settings_digest != stamp.settings_digest
    {
        return Err("Task checkout scope changed".into());
    }
    store
        .validate_native_checkout_stamp_in_transaction(stamp)
        .map_err(|_| "Source checkout authority changed; reapprove task")?;
    let revision = store
        .linear_revision(&user.id)
        .map_err(|_| "Linear configuration unavailable")?
        .ok_or("Linear was disconnected")?;
    if revision != record.task.linear_revision {
        return Err("Linear configuration changed; reapprove task".into());
    }
    let raw = store
        .project_settings(&record.job.project_id)
        .map_err(|_| "Project settings unavailable")?
        .unwrap_or_else(|| "{}".into());
    if crate::auth::hash_token(&raw) != record.task.settings_digest {
        return Err("Project settings changed; reapprove task".into());
    }
    let settings: dunlin_domain::ProjectSettings =
        serde_json::from_str(&raw).map_err(|_| "Invalid project settings")?;
    if settings.permission_mode == Some(dunlin_domain::PermissionMode::ReadOnly)
        && record.job.profile.allow_tools
    {
        return Err("Read-only policy disallows approved tools".into());
    }
    Ok(())
}
async fn capture_checkout(
    state: &AppState,
    project: &str,
    user: &str,
    permission: MetadataPermission,
    expected: Option<&NativeCheckoutStamp>,
    deadline: tokio::time::Instant,
) -> Result<NativeCheckoutAuthority, String> {
    let candidate = state
        .store
        .lock()
        .map_err(|_| "Store unavailable")?
        .native_checkout_candidate(project, user, permission)
        .map_err(|e| e.to_string())?;
    let permit = state
        .fs_permit()
        .map_err(|_| "Checkout validation capacity reached")?;
    let job = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        candidate.validate_filesystem()
    });
    let validated = tokio::time::timeout_at(
        deadline.min(tokio::time::Instant::now() + Duration::from_secs(5)),
        job,
    )
    .await
    .map_err(|_| "Checkout filesystem validation timed out")?
    .map_err(|_| "Checkout filesystem validation interrupted")?
    .map_err(|e| e.to_string())?;
    let authority = state
        .store
        .lock()
        .map_err(|_| "Store unavailable")?
        .authorize_native_checkout(validated)
        .map_err(|e| e.to_string())?;
    if expected.is_some_and(|stamp| stamp != &authority.stamp()) {
        return Err("Persisted checkout stamp changed; explicit reapproval required".into());
    }
    Ok(authority)
}
/// Reconstruct only by inspecting current SQL/FS and comparing every persisted
/// pin. This helper never refreshes the caller's saved approval or starts work.
pub(crate) async fn restore_job_checkout(
    state: &AppState,
    stamp: &NativeCheckoutStamp,
    deadline: tokio::time::Instant,
) -> Result<NativeCheckoutAuthority, String> {
    capture_checkout(
        state,
        &stamp.native_grant.project_id,
        &stamp.native_grant.user_id,
        stamp.permission,
        Some(stamp),
        deadline,
    )
    .await
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
    let (project, native_grant) = {
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store lock poisoned"))?;
        let project = store
            .project_for_user(&request.project_id, &auth.user.id)?
            .ok_or_else(ApiError::forbidden)?;
        let grant = store.capture_native_grant(&project.id, &auth.user.id)?;
        (project, grant)
    };
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
    let source = capture_checkout(
        &state,
        &project.id,
        &auth.user.id,
        MetadataPermission::Read,
        None,
        tokio::time::Instant::now() + Duration::from_secs(10),
    )
    .await
    .map_err(ApiError::conflict)?;
    let root = source.root().to_owned();
    let head = runner::approved_head(&root)
        .await
        .map_err(ApiError::bad_request)?;
    let source = capture_checkout(
        &state,
        &project.id,
        &auth.user.id,
        MetadataPermission::Read,
        Some(&source.stamp()),
        tokio::time::Instant::now() + Duration::from_secs(5),
    )
    .await
    .map_err(ApiError::conflict)?;
    let task = Task {
        source_checkout_stamp: Some(source.stamp()),
        native_grant: Some(native_grant),
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
        checkout_grant: None,
        checkout_stamp: None,
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
    let job = {
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store lock poisoned"))?;
        store
            .native_checkout_transaction(|store| {
                authorized_in_store(&state, store, &provisional)
                    .map_err(crate::store::StoreError::Conflict)?;
                store.revalidate_native_checkout_in_transaction(&source)?;
                store.enqueue_in_transaction(&auth.user.id, &provisional.task)
            })
            .map_err(|e| ApiError::conflict(e.to_string()))?
    };
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
