//! Registry/clock unit tests. Actual Accepted/WSS construction is separately
//! exercised by worker_fixture; these tests do not claim network verification.
use super::*;
use crate::{config::Config, store::Store};
use serde_json::json;
struct Fixture {
    state: AppState,
    owner: String,
    workspace: String,
    host: String,
    identity: WorkerIdentity,
    lease: ChannelLease,
}
impl Fixture {
    fn new() -> Self {
        let store = Store::open_in_memory().unwrap();
        let owner = store.create_user("health-clock-owner", "admin").unwrap();
        let workspace = store.create_workspace(&owner, "Health clocks").unwrap();
        let pending = store.create_host(&owner.id, &workspace.id, serde_json::from_value(json!({"name":"Clock peer","node_id":"clock-worker","serve_origin":"https://worker.tail123.ts.net","capability_policy":{"max_parallel":1,"harnesses":["opencode"]}})).unwrap()).unwrap();
        let credential = store.enroll_host(&pending.enrollment.ticket, serde_json::from_value(json!({"protocol_version":1,"version":"clock-test","os":"linux","architecture":"aarch64","harnesses":["opencode"],"sandbox":"none","max_parallel":1})).unwrap()).unwrap();
        store
            .approve_host(&owner.id, &workspace.id, &credential.host_id, 2)
            .unwrap();
        let identity = store
            .authenticate_worker_token(&credential.credential)
            .unwrap();
        let state = AppState::new(store, Config::default());
        let lease = state
            .store
            .lock()
            .unwrap()
            .open_worker_channel(
                &identity,
                &state.worker_channels.boot_id,
                "old-worker",
                "old-channel",
                now_ms().unwrap(),
                TTL_MS,
            )
            .unwrap();
        Self {
            state,
            owner: owner.id,
            workspace: workspace.id,
            host: credential.host_id,
            identity,
            lease,
        }
    }
    fn observe(&self) -> Option<LiveChannelObservation> {
        self.state
            .store
            .lock()
            .unwrap()
            .host_health_observation(
                &self.owner,
                &self.workspace,
                &self.host,
                &self.state.worker_channels,
            )
            .unwrap()
            .connection
    }
    fn renew(&self, sequence: u64, received: Instant) {
        let renewed = self
            .state
            .store
            .lock()
            .unwrap()
            .renew_worker_channel(
                &self.identity,
                &self.lease,
                sequence,
                now_ms().unwrap(),
                TTL_MS,
            )
            .unwrap();
        self.state
            .worker_channels
            .observed_heartbeat(&renewed, received);
    }
}
#[test]
fn host_health_duplicate_and_late_sql_completion_cannot_refresh_monotonic_liveness() {
    let f = Fixture::new();
    assert!(f.observe().is_none());
    let _registration = f
        .state
        .worker_channels
        .register_health(&f.state, &f.identity, &f.lease, 1)
        .unwrap();
    f.renew(1, Instant::now());
    assert!(f.observe().is_some());
    let connection = f
        .state
        .worker_channels
        .health
        .connections
        .lock()
        .unwrap()
        .get(&f.host)
        .unwrap()
        .clone();
    connection.pulse.lock().unwrap().received = Instant::now() - Duration::from_secs(20);
    assert!(f.observe().is_none());
    f.renew(1, Instant::now()); // SQL duplicate is valid; observational clock stays old.
    assert!(f.observe().is_none());
    f.renew(2, Instant::now() - Duration::from_secs(11));
    assert_eq!(connection.pulse.lock().unwrap().sequence, 2);
    assert!(
        f.observe().is_none(),
        "fresh SQL deadline cannot disguise old received heartbeat"
    );
    f.renew(3, Instant::now());
    assert_eq!(f.observe().unwrap().heartbeat_sequence, 3);
}
#[test]
fn host_health_old_channel_drop_or_heartbeat_cannot_remove_or_refresh_its_replacement() {
    let f = Fixture::new();
    let old = f
        .state
        .worker_channels
        .register_health(&f.state, &f.identity, &f.lease, 1)
        .unwrap();
    let new = f
        .state
        .store
        .lock()
        .unwrap()
        .open_worker_channel(
            &f.identity,
            &f.state.worker_channels.boot_id,
            "new-worker",
            "new-channel",
            now_ms().unwrap(),
            TTL_MS,
        )
        .unwrap();
    let current = f
        .state
        .worker_channels
        .register_health(&f.state, &f.identity, &new, 2)
        .unwrap();
    drop(old);
    assert_eq!(f.observe().unwrap().protocol_version, 2);
    let mut stale = f.lease.clone();
    stale.last_sequence = 99;
    f.state
        .worker_channels
        .observed_heartbeat(&stale, Instant::now());
    assert_eq!(f.observe().unwrap().heartbeat_sequence, 0);
    drop(current);
    assert!(
        f.observe().is_none(),
        "active SQL row without live registration remains unconfirmed"
    );
}
