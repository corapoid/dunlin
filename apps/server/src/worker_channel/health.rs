//! Observations of actual accepted WSS lifetimes. SQL rows alone never create
//! these registrations. Nothing here can authorize execution or renew a lease.
use super::*;
use crate::store::Store;
use serde::Serialize;
use std::{collections::BTreeMap, sync::Mutex, time::Instant};

struct Pulse {
    sequence: u64,
    received: Instant,
}
struct Connection {
    identity: WorkerIdentity,
    lease: ChannelLease,
    version: u16,
    alive: AtomicBool,
    pulse: Mutex<Pulse>,
}
#[derive(Default)]
pub(super) struct Registry {
    connections: Mutex<BTreeMap<String, Arc<Connection>>>,
}
pub(super) struct Registration {
    registry: Arc<Service>,
    connection: Arc<Connection>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.connection.alive.store(false, Ordering::Release);
        if let Ok(mut connections) = self.registry.health.connections.lock()
            && connections
                .get(self.connection.identity.host_id())
                .is_some_and(|current| Arc::ptr_eq(current, &self.connection))
        {
            connections.remove(self.connection.identity.host_id());
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct LiveChannelObservation {
    pub protocol_version: u16,
    pub heartbeat_sequence: u64,
    pub heartbeat_age_ms: u64,
    pub lease_remaining_ms: u64,
}
impl Service {
    pub(super) fn register_health(
        self: &Arc<Self>,
        state: &AppState,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        version: u16,
    ) -> Result<Registration> {
        let store = state
            .store
            .lock()
            .map_err(|_| WorkerErrorCode::InternalError)?;
        let current = store
            .validate_worker_channel(identity, lease, &self.boot_id, now_ms()?)
            .map_err(|_| WorkerErrorCode::StaleLease)?;
        let mut connections = self
            .health
            .connections
            .lock()
            .map_err(|_| WorkerErrorCode::InternalError)?;
        if connections
            .get(identity.host_id())
            .is_some_and(|connection| connection.lease.epoch >= current.epoch)
        {
            return Err(WorkerErrorCode::StaleLease);
        }
        let connection = Arc::new(Connection {
            identity: identity.clone(),
            lease: current,
            version,
            alive: AtomicBool::new(true),
            pulse: Mutex::new(Pulse {
                sequence: 0,
                received: Instant::now(),
            }),
        });
        connections.insert(identity.host_id().to_owned(), connection.clone());
        Ok(Registration {
            registry: self.clone(),
            connection,
        })
    }
    /// Only after the actual heartbeat SQL transaction commits. Keep receive
    /// time, not completion time: a stalled SQL job must not look freshly alive.
    pub(super) fn observed_heartbeat(&self, lease: &ChannelLease, received: Instant) {
        let connection = self
            .health
            .connections
            .lock()
            .ok()
            .and_then(|connections| connections.get(&lease.host_id).cloned());
        if let Some(connection) = connection
            && connection.alive.load(Ordering::Acquire)
            && connection.lease.channel_id == lease.channel_id
            && connection.lease.epoch == lease.epoch
            && connection.lease.controller_boot_id == lease.controller_boot_id
            && connection.lease.worker_boot_id == lease.worker_boot_id
            && let Ok(mut pulse) = connection.pulse.lock()
            && lease.last_sequence > pulse.sequence
            && received >= pulse.received
        {
            // Duplicate heartbeats cannot extend this observational clock.
            *pulse = Pulse {
                sequence: lease.last_sequence,
                received,
            };
        }
    }
    /// Caller owns the Store transaction, including its manager/scope check.
    /// A serialized result is only a transient observation, never a grant.
    pub(crate) fn live_channel_observation(
        &self,
        store: &Store,
        host: &str,
        now: i64,
    ) -> Option<LiveChannelObservation> {
        if self.shutdown.load(Ordering::Acquire) {
            return None;
        }
        let connection = self.health.connections.lock().ok()?.get(host).cloned()?;
        if !connection.alive.load(Ordering::Acquire) {
            return None;
        }
        let lease = store
            .validate_worker_channel_in_transaction(
                &connection.identity,
                &connection.lease,
                &self.boot_id,
                now,
            )
            .ok()?;
        let pulse = connection.pulse.lock().ok()?;
        let age = u64::try_from(pulse.received.elapsed().as_millis()).ok()?;
        if age >= TTL_MS
            || pulse.sequence != lease.last_sequence
            || lease.last_sequence > worker::MAX_EXECUTION_INTEGER
            || !connection.alive.load(Ordering::Acquire)
        {
            return None;
        }
        Some(LiveChannelObservation {
            protocol_version: connection.version,
            heartbeat_sequence: pulse.sequence,
            heartbeat_age_ms: age,
            lease_remaining_ms: (TTL_MS - age)
                .min(u64::try_from(lease.expires_at_ms.checked_sub(now)?).ok()?),
        })
    }
}

#[cfg(test)]
mod tests;
