//! Cheap context proof tests only; no process or sandbox is started here.
use super::*;
use crate::worker_client::broker::fixture;
use wire::{ChannelFence, session::broker::BrokerLimits};

fn fence() -> ChannelFence {
    ChannelFence {
        channel_id: "channel".into(),
        controller_boot_id: "controller".into(),
        worker_boot_id: "worker".into(),
    }
}
fn scope() -> local::ExportScope {
    local::ExportScope {
        controller_node_id: "controller-node".into(),
        worker_node_id: "worker-node".into(),
        workspace_id: "workspace".into(),
        host_id: "host".into(),
        enrollment_epoch: 1,
        credential_revision: 1,
    }
}
#[test]
fn broker_context_requires_exact_live_owner_and_does_not_revive_after_drop() {
    let (client, owner) = fixture::pair(
        fence(),
        BrokerLimits {
            max_pending: 2,
            max_reply_bytes: 65536,
        },
    );
    let connection = Connected {
        fence: fence(),
        guard: Arc::new(|| Ok(())),
        open: Arc::new(AtomicBool::new(true)),
        broker: Some(client.clone()),
    };
    let scope = scope();
    assert_eq!(client.limits().max_pending, 2);
    assert!(
        connection
            .context(&scope, &|| connection.authorize())
            .is_ok()
    );
    let mut other = connection.clone();
    other.fence.worker_boot_id = "replacement".into();
    assert!(other.authorize().is_err());
    assert!(other.context(&scope, &|| Ok(())).is_err());
    drop(owner);
    assert!(connection.authorize().is_err());
    assert!(connection.context(&scope, &|| Ok(())).is_err());
    connection.open.store(true, Ordering::Release);
    assert!(connection.authorize().is_err());
}
#[test]
fn connected_open_guard_is_independent_of_a_still_live_broker_owner() {
    let (client, _owner) = fixture::pair(
        fence(),
        BrokerLimits {
            max_pending: 1,
            max_reply_bytes: 65536,
        },
    );
    let connection = Connected {
        fence: fence(),
        guard: Arc::new(|| Ok(())),
        open: Arc::new(AtomicBool::new(true)),
        broker: Some(client),
    };
    assert!(connection.authorize().is_ok());
    connection.open.store(false, Ordering::Release);
    assert!(connection.authorize().is_err());
    assert!(connection.context(&scope(), &|| Ok(())).is_err());
}
