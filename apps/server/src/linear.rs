//! Per-user, host-owned Linear MCP. No credential value is persisted in SQLite,
//! returned over the relay API, injected into ACP, or exposed to vault MCP.
use crate::api::{ApiError, AppState, AuthUser};
use axum::{
    Json,
    extract::State,
    http::{
        StatusCode,
        header::{CACHE_CONTROL, HeaderValue},
    },
    response::{IntoResponse, Response},
};
use dunlin_mcp_client::{
    Client,
    linear::{self, Filter, Inbox},
};
use dunlin_vault::store::{SecretStore, ServiceKeychainStore};
use serde::Deserialize;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use zeroize::Zeroizing;

pub(crate) struct Service {
    secrets: Arc<dyn SecretStore>,
    endpoint: String,
    slots: Arc<Semaphore>,
    busy: Arc<Mutex<HashSet<String>>>,
}

impl Default for Service {
    fn default() -> Self {
        Self {
            secrets: Arc::new(ServiceKeychainStore::relay()),
            endpoint: linear::ENDPOINT.into(),
            slots: Arc::new(Semaphore::new(2)),
            busy: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

struct Guard {
    user: String,
    busy: Arc<Mutex<HashSet<String>>>,
    _slot: OwnedSemaphorePermit,
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Ok(mut busy) = self.busy.lock() {
            busy.remove(&self.user);
        }
    }
}
impl Service {
    fn guard(&self, user: &str) -> Result<Guard, ApiError> {
        let slot = self.slots.clone().try_acquire_owned().map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Linear connections are busy; try again",
            )
        })?;
        let mut busy = self
            .busy
            .lock()
            .map_err(|_| ApiError::internal("Linear lock poisoned"))?;
        if !busy.insert(user.into()) {
            return Err(ApiError::conflict(
                "Another Linear operation is running for this user",
            ));
        }
        Ok(Guard {
            user: user.into(),
            busy: self.busy.clone(),
            _slot: slot,
        })
    }

    fn fetch(&self, token: &str, filter: &Filter) -> Result<Vec<linear::Issue>, String> {
        let mut client =
            Client::connect(&self.endpoint, token).map_err(|error| error.to_string())?;
        let result = client
            .tools()
            .and_then(|tools| linear::issues(&mut client, &tools, filter))
            .map_err(|error| error.to_string());
        let _ = client.close();
        result
    }

    #[cfg(test)]
    pub(crate) fn testing(secrets: Arc<dyn SecretStore>, endpoint: String) -> Self {
        assert!(endpoint.starts_with("http://127.0.0.1:"));
        Self {
            secrets,
            endpoint,
            ..Self::default()
        }
    }
}

#[derive(Debug)]
pub(crate) struct DeliveryError {
    pub retry: bool,
    pub message: String,
}
fn job_status_sql_authorized(
    state: &AppState,
    record: &crate::store::jobs::Record,
) -> Result<(), String> {
    let host_accounts = crate::management::host_accounts(&state.config);
    let store = state.store.lock().map_err(|_| "Store unavailable")?;
    store
        .native_checkout_transaction::<_, crate::store::StoreError>(|store| {
            let denied = || {
                crate::store::StoreError::Conflict(
                    "Job authority changed; inspect status manually".into(),
                )
            };
            let user = store
                .active_device_user(&record.task.device)?
                .filter(|u| u.id == record.job.owner)
                .ok_or_else(denied)?;
            if (user.role != "admin" && host_accounts)
                || !store.oidc_device_allowed(&record.task.device, state.config.oidc.as_ref())?
            {
                return Err(denied());
            }
            let source = record
                .task
                .source_checkout_stamp
                .as_ref()
                .ok_or_else(denied)?;
            if source.native_grant.project_id != record.job.project_id
                || source.native_grant.user_id != record.job.owner
                || record.task.native_grant.as_ref() != Some(&source.native_grant)
                || source.settings_digest != record.task.settings_digest
            {
                return Err(denied());
            }
            store.validate_native_checkout_stamp_in_transaction(source)?;
            match (&record.job.checkout_project, &record.checkout_stamp) {
                (Some(project), Some(stamp))
                    if stamp.native_grant.project_id == *project
                        && stamp.native_grant.user_id == record.job.owner
                        && record.checkout_grant.as_ref() == Some(&stamp.native_grant) =>
                {
                    store.validate_native_checkout_stamp_in_transaction(stamp)?
                }
                (None, None) if record.job.status != dunlin_domain::jobs::Status::Review => {}
                _ => return Err(denied()),
            }
            let current = store.job(&record.job.id)?.ok_or_else(denied)?;
            if current.job.cancel_requested
                || current.job.status != record.job.status
                || current.job.owner != record.job.owner
                || current.job.project_id != record.job.project_id
                || current.task.source_checkout_stamp != record.task.source_checkout_stamp
                || current.checkout_stamp != record.checkout_stamp
            {
                return Err(denied());
            }
            if store.linear_revision(&record.job.owner)?.as_deref()
                != Some(record.task.linear_revision.as_str())
            {
                return Err(crate::store::StoreError::Conflict(
                    "Linear authorization changed since approval; update the issue manually".into(),
                ));
            }
            Ok(())
        })
        .map_err(|error| match error {
            crate::store::StoreError::Conflict(message)
                if message
                    == "Linear authorization changed since approval; update the issue manually" =>
            {
                message
            }
            _ => "Native job authority changed since approval; inspect status manually".into(),
        })
}

fn job_status_checkout_current(
    state: &AppState,
    stamp: &crate::store::native_checkout::NativeCheckoutStamp,
) -> Result<(), String> {
    let permit = state
        .fs_permit()
        .map_err(|_| "Checkout validation is busy; inspect status manually")?;
    let candidate = {
        let store = state.store.lock().map_err(|_| "Store unavailable")?;
        store
            .native_checkout_candidate(
                &stamp.native_grant.project_id,
                &stamp.native_grant.user_id,
                stamp.permission,
            )
            .map_err(|_| "Approved checkout authority is unavailable")?
    };
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    // No runtime handle is needed by this synchronous MCP worker. A timed-out
    // filesystem job retains its permit until it really completes.
    std::thread::Builder::new()
        .name("dunlin-status-checkout".into())
        .spawn(move || {
            let _permit = permit;
            let _ = send.send(candidate.validate_filesystem());
        })
        .map_err(|_| "Checkout validation worker unavailable")?;
    let validated = receive
        .recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| "Checkout validation expired; inspect status manually")?
        .map_err(|_| "Approved checkout identity changed; inspect status manually")?;
    let store = state.store.lock().map_err(|_| "Store unavailable")?;
    let authority = store
        .authorize_native_checkout(validated)
        .map_err(|_| "Approved checkout authority changed")?;
    if authority.stamp() != *stamp {
        return Err("Approved checkout stamp changed; inspect status manually".into());
    }
    Ok(())
}

fn native_job_status_authorized(
    state: &AppState,
    record: &crate::store::jobs::Record,
) -> Result<(), String> {
    job_status_sql_authorized(state, record)?;
    let source = record
        .task
        .source_checkout_stamp
        .as_ref()
        .ok_or("Legacy task has no pinned checkout authority")?;
    job_status_checkout_current(state, source)?;
    if let Some(derived) = &record.checkout_stamp {
        job_status_checkout_current(state, derived)?;
    }
    job_status_sql_authorized(state, record)
}
pub(crate) fn job_status(
    state: &AppState,
    record: &crate::store::jobs::Record,
) -> Result<(), DeliveryError> {
    let retry = |message: String| DeliveryError {
        retry: true,
        message,
    };
    let review = |message: String| DeliveryError {
        retry: false,
        message,
    };
    native_job_status_authorized(state, record).map_err(review)?;
    let user = &record.job.owner;
    authorized(state, user, None)
        .map_err(|_| review("User authorization no longer permits status delivery".into()))?;
    state
        .project_for_user(user, &record.job.project_id)
        .map_err(|_| review("Project grant was revoked before status delivery".into()))?;
    let _guard = state.linear.guard(user).map_err(|_| {
        retry("Linear is busy; status delivery will retry before making a mutation".into())
    })?;
    let connection = state
        .store
        .lock()
        .map_err(|_| review("Store unavailable".into()))?
        .linear_connection(user)
        .map_err(|_| review("Linear configuration unavailable".into()))?
        .ok_or_else(|| {
            review("Linear was disconnected; reconnect and explicitly retry status delivery".into())
        })?;
    if connection.revision != record.task.linear_revision {
        return Err(review(
            "Linear authorization changed since approval; update the issue manually".into(),
        ));
    }
    let token = state
        .linear
        .secrets
        .get(&connection.credential_id)
        .map_err(|_| retry("Unlock the relay keyring to deliver the review status".into()))?;
    native_job_status_authorized(state, record).map_err(review)?;
    let mut client = Client::connect(&state.linear.endpoint, &token)
        .map_err(|error| retry(error.to_string()))?;
    let result = (|| {
        let tools = client.tools().map_err(|error| retry(error.to_string()))?;
        let current = linear::get_issue(&mut client, &tools, &record.task.issue.id)
            .map_err(|error| retry(error.to_string()))?;
        native_job_status_authorized(state, record).map_err(review)?;
        if current.status == record.job.profile.review_state {
            return Ok(());
        }
        if current.status != record.task.issue.status
            || !current
                .labels
                .iter()
                .any(|label| label.eq_ignore_ascii_case(&connection.filter.label))
        {
            return Err(review("Issue status/label changed since approval. Inspect Linear before explicitly retrying or updating it manually.".into()));
        }
        authorized(state, user, None)
            .map_err(|_| review("Authorization revoked before status mutation".into()))?;
        state
            .project_for_user(user, &record.job.project_id)
            .map_err(|_| review("Project grant revoked before status mutation".into()))?;
        native_job_status_authorized(state, record).map_err(review)?;
        linear::set_status(
            &mut client,
            &tools,
            &record.task.issue.id,
            &record.job.profile.review_state,
        )
        .map_err(|error| review(error.to_string()))?;
        native_job_status_authorized(state,record).map_err(|_|review("Native authority changed during status mutation; inspect Linear before reconciling".into()))
    })();
    let _ = client.close();
    result
}

/// Recheck after slow MCP/Keychain work as well as before it. Background sync
/// uses the current relay user, never a disconnected desktop's device token.
fn authorized(state: &AppState, user: &str, device: Option<&str>) -> Result<(), ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store lock poisoned"))?;
    let current = if let Some(device) = device {
        store.active_device_user(device)?
    } else {
        store.find_user(user)?
    }
    .filter(|current| current.id == user)
    .ok_or_else(ApiError::unauthorized)?;
    if current.role != "admin" && crate::management::host_accounts(&state.config) {
        return Err(ApiError::forbidden());
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Connect {
    filter: Filter,
    token: String,
}

fn private(inbox: Inbox) -> Response {
    let mut response = Json(inbox).into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(crate) async fn inbox(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Response, ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store lock poisoned"))?;
    Ok(private(
        store
            .linear_connection(&auth.user.id)?
            .map(|connection| connection.inbox)
            .unwrap_or_default(),
    ))
}

pub(crate) async fn connect(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(mut request): Json<Connect>,
) -> Result<Response, ApiError> {
    request.filter.validate().map_err(|_| {
        ApiError::bad_request(
            "Choose a non-empty label and an optional team (at most 128 characters)",
        )
    })?;
    if request.token.is_empty()
        || request.token.len() > 8192
        || !request
            .token
            .bytes()
            .all(|byte| (0x21..=0x7e).contains(&byte))
    {
        return Err(ApiError::bad_request("Invalid Linear token"));
    }
    let token = Zeroizing::new(std::mem::take(&mut request.token));
    let guard = state.linear.guard(&auth.user.id)?;
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        authorized(&state, &auth.user.id, Some(&auth.device.id))?;
        let issues = state
            .linear
            .fetch(&token, &request.filter)
            .map_err(|error| ApiError::new(StatusCode::BAD_GATEWAY, error))?;
        authorized(&state, &auth.user.id, Some(&auth.device.id))?;
        let credential_id = format!("linear-{}", uuid::Uuid::now_v7());
        state
            .linear
            .secrets
            .set(&credential_id, &token)
            .map_err(|_| {
                ApiError::internal(
                    "Could not save Linear token in the relay keyring; nothing was configured",
                )
            })?;
        let inbox = Inbox {
            filter: Some(request.filter),
            issues,
            synced_at: Some(crate::store::now_secs()),
            error: None,
        };
        let save = (|| {
            authorized(&state, &auth.user.id, Some(&auth.device.id))?;
            state
                .store
                .lock()
                .map_err(|_| ApiError::internal("Store lock poisoned"))?
                .linear_save(
                    &auth.user.id,
                    &credential_id,
                    &uuid::Uuid::now_v7().to_string(),
                    &inbox,
                )?;
            Ok::<_, ApiError>(())
        })();
        if let Err(error) = save {
            if state.linear.secrets.delete(&credential_id).is_err()
                && let Ok(store) = state.store.lock()
            {
                let _ = store.linear_retire(&credential_id);
            }
            return Err(error);
        }
        cleanup(&state);
        authorized(&state, &auth.user.id, Some(&auth.device.id))?;
        Ok(private(inbox))
    })
    .await
    .map_err(|_| ApiError::internal("Linear connection worker stopped"))?
}

fn sync_inner(state: &AppState, user: &str, device: Option<&str>) -> Result<Inbox, ApiError> {
    authorized(state, user, device)?;
    let connection = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store lock poisoned"))?
        .linear_connection(user)?
        .ok_or_else(ApiError::not_found)?;
    let result = state
        .linear
        .secrets
        .get(&connection.credential_id)
        .map_err(|_| "Linear token unavailable; unlock the relay keyring or reconnect".to_string())
        .and_then(|token| state.linear.fetch(&token, &connection.filter));
    authorized(state, user, device)?;
    let mut inbox = connection.inbox;
    match result {
        Ok(issues) => {
            inbox.issues = issues;
            inbox.synced_at = Some(crate::store::now_secs());
            inbox.error = None;
        }
        Err(error) => inbox.error = Some(error), // Keep the last complete snapshot, explicitly stale.
    }
    if !state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store lock poisoned"))?
        .linear_update(user, &connection.revision, &inbox)?
    {
        return Err(ApiError::conflict(
            "Linear configuration changed during synchronization",
        ));
    }
    Ok(inbox)
}

pub(crate) async fn sync(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Response, ApiError> {
    let guard = state.linear.guard(&auth.user.id)?;
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        sync_inner(&state, &auth.user.id, Some(&auth.device.id)).map(private)
    })
    .await
    .map_err(|_| ApiError::internal("Linear synchronization worker stopped"))?
}

pub(crate) async fn disconnect(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Response, ApiError> {
    let guard = state.linear.guard(&auth.user.id)?;
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        authorized(&state, &auth.user.id, Some(&auth.device.id))?;
        state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store lock poisoned"))?
            .linear_remove(&auth.user.id)?;
        cleanup(&state);
        Ok(private(Inbox::default()))
    })
    .await
    .map_err(|_| ApiError::internal("Linear disconnect worker stopped"))?
}

fn cleanup(state: &AppState) {
    let ids = state
        .store
        .lock()
        .ok()
        .and_then(|store| store.linear_cleanup().ok())
        .unwrap_or_default();
    for id in ids.into_iter().take(4) {
        if id
            .strip_prefix("linear-")
            .is_none_or(|id| uuid::Uuid::parse_str(id).is_err())
        {
            continue;
        }
        if state.linear.secrets.delete(&id).is_ok()
            && let Ok(store) = state.store.lock()
        {
            let _ = store.linear_cleaned(&id);
        }
    }
}

pub(crate) fn start_background(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            background_once(state.clone()).await;
        }
    });
}

pub(crate) async fn background_once(state: AppState) {
    let read = state.clone();
    let due = tokio::task::spawn_blocking(move || {
        cleanup(&read);
        read.store
            .lock()
            .ok()
            .and_then(|store| store.linear_due().ok())
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    let mut running = Vec::new();
    for user in due {
        let Ok(guard) = state.linear.guard(&user) else {
            continue;
        };
        let state = state.clone();
        running.push(tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let _ = sync_inner(&state, &user, None);
        }));
    }
    for worker in running {
        let _ = worker.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dunlin_vault::store::MemoryStore;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct DelayedDelete {
        memory: MemoryStore,
        ready: AtomicBool,
    }
    impl SecretStore for DelayedDelete {
        fn get(&self, id: &str) -> Result<Zeroizing<String>, String> {
            self.memory.get(id)
        }
        fn set(&self, id: &str, value: &str) -> Result<(), String> {
            self.memory.set(id, value)
        }
        fn delete(&self, id: &str) -> Result<(), String> {
            if self.ready.load(Ordering::SeqCst) {
                self.memory.delete(id)
            } else {
                Err("keyring locked".into())
            }
        }
        fn get_policy(&self, id: &str) -> Result<Zeroizing<String>, String> {
            self.memory.get_policy(id)
        }
        fn set_policy(&self, id: &str, value: &str) -> Result<(), String> {
            self.memory.set_policy(id, value)
        }
        fn delete_policy(&self, id: &str) -> Result<(), String> {
            self.memory.delete_policy(id)
        }
    }

    #[test]
    fn credential_cleanup_survives_a_locked_keyring_and_retries() {
        let store = crate::store::Store::open_in_memory().unwrap();
        let user = store.create_user("member", "member").unwrap();
        let id = format!("linear-{}", uuid::Uuid::now_v7());
        let secrets = Arc::new(DelayedDelete {
            memory: MemoryStore::new(),
            ready: AtomicBool::new(false),
        });
        secrets.set(&id, "fixture-key").unwrap();
        store
            .linear_save(
                &user.id,
                &id,
                "revision",
                &Inbox {
                    filter: Some(Filter {
                        label: "dunlin".into(),
                        team: None,
                    }),
                    ..Inbox::default()
                },
            )
            .unwrap();
        store.linear_remove(&user.id).unwrap();
        let mut state = AppState::new(store, crate::config::Config::default());
        state.linear = Arc::new(Service::testing(
            secrets.clone(),
            "http://127.0.0.1:1/mcp".into(),
        ));
        cleanup(&state);
        assert!(secrets.get(&id).is_ok());
        assert_eq!(
            state.store.lock().unwrap().linear_cleanup().unwrap(),
            vec![id.clone()]
        );
        secrets.ready.store(true, Ordering::SeqCst);
        cleanup(&state);
        assert!(secrets.get(&id).is_err());
        assert!(
            state
                .store
                .lock()
                .unwrap()
                .linear_cleanup()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn operations_are_user_serialized_and_concurrency_is_bounded() {
        let service = Service::testing(
            Arc::new(MemoryStore::new()),
            "http://127.0.0.1:1/mcp".into(),
        );
        let first = service.guard("first").unwrap();
        assert!(service.guard("first").is_err());
        let second = service.guard("second").unwrap();
        assert!(service.guard("third").is_err());
        drop(first);
        assert!(service.guard("third").is_ok());
        drop(second);
        assert!(service.guard("first").is_ok());
    }
}
