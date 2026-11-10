//! Explicit desktop heartbeats, not historical API activity or agent leases.
use super::{ApiError, AppState, AuthUser};
use axum::{
    Json,
    extract::State,
    http::{
        StatusCode,
        header::{CACHE_CONTROL, HeaderValue},
    },
    response::{IntoResponse, Response},
};
use dunlin_domain::presence::{PRESENCE_TTL_SECS, Snapshot, User};
use std::{
    collections::{BTreeMap, HashMap},
    time::{Duration, Instant},
};

const MAX_DEVICES: usize = 512;

#[derive(Default)]
pub(super) struct Registry {
    devices: HashMap<String, Instant>,
}

impl Registry {
    fn heartbeat(&mut self, device: &str, now: Instant) -> Result<(), ApiError> {
        self.devices
            .retain(|_, seen| now.duration_since(*seen) < Duration::from_secs(PRESENCE_TTL_SECS));
        if self.devices.len() >= MAX_DEVICES && !self.devices.contains_key(device) {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many connected devices",
            ));
        }
        self.devices.insert(device.into(), now);
        Ok(())
    }
}

pub(super) async fn heartbeat(
    State(state): State<AppState>,
    AuthUser { user, device }: AuthUser,
) -> Result<Response, ApiError> {
    let mut registry = state
        .presence
        .lock()
        .map_err(|_| ApiError::internal("Presence lock poisoned"))?;
    registry.heartbeat(&device.id, Instant::now())?;
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store lock poisoned"))?;
    let admin_only = crate::management::host_accounts(&state.config);
    let mut users: BTreeMap<String, User> = BTreeMap::new();
    let mut invalid = Vec::new();
    for id in registry.devices.keys() {
        let Some(peer) = store.active_device_user(id)? else {
            invalid.push(id.clone());
            continue;
        };
        if admin_only && peer.role != "admin" {
            invalid.push(id.clone());
            continue;
        }
        users
            .entry(peer.id.clone())
            .and_modify(|peer| peer.devices += 1)
            .or_insert(User {
                id: peer.id,
                name: peer.name,
                devices: 1,
            });
    }
    for id in invalid {
        registry.devices.remove(&id);
    }
    let mut response = Json(Snapshot {
        workload: Some(store.job_workload(&user.id)?),
        self_user_id: user.id,
        users: users.into_values().collect(),
        ttl_secs: PRESENCE_TTL_SECS,
    })
    .into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_expires_and_heartbeats_do_not_duplicate_devices() {
        let mut registry = Registry::default();
        let start = Instant::now();
        registry.heartbeat("first", start).unwrap();
        registry
            .heartbeat("first", start + Duration::from_secs(5))
            .unwrap();
        assert_eq!(registry.devices.len(), 1);
        registry
            .heartbeat("second", start + Duration::from_secs(34))
            .unwrap();
        assert_eq!(registry.devices.len(), 2);
        registry
            .heartbeat("second", start + Duration::from_secs(35))
            .unwrap();
        assert_eq!(registry.devices.len(), 1);
        assert!(!registry.devices.contains_key("first"));
    }

    #[test]
    fn presence_is_bounded_without_evicting_live_connections() {
        let mut registry = Registry::default();
        let now = Instant::now();
        for id in 0..MAX_DEVICES {
            registry.heartbeat(&id.to_string(), now).unwrap();
        }
        assert_eq!(
            registry.heartbeat("overflow", now).unwrap_err().status,
            StatusCode::TOO_MANY_REQUESTS
        );
        registry.heartbeat("0", now).unwrap();
        registry
            .heartbeat("new", now + Duration::from_secs(PRESENCE_TTL_SECS))
            .unwrap();
        assert_eq!(registry.devices.len(), 1);
    }
}
