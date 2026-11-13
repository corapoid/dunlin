//! Test-only observation of a client created by the verified production loop.
//! This does not construct native execution authority or an MCP account.
use super::*;
use std::sync::{Mutex, OnceLock};

fn registry() -> &'static Mutex<BTreeMap<String, Client>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<String, Client>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}
pub(crate) struct Registration {
    host: String,
    fence: ChannelFence,
}
impl Drop for Registration {
    fn drop(&mut self) {
        let mut registry = registry().lock().unwrap();
        if registry
            .get(&self.host)
            .is_some_and(|client| client.fence == self.fence)
        {
            registry.remove(&self.host);
        }
    }
}
pub(in crate::worker_client) fn publish(host: &str, owner: &SocketOwner) -> Registration {
    registry()
        .lock()
        .unwrap()
        .insert(host.into(), owner.client.clone());
    Registration {
        host: host.into(),
        fence: owner.client.fence.clone(),
    }
}
pub(crate) fn get(host: &str, fence: &ChannelFence) -> Option<Client> {
    registry()
        .lock()
        .unwrap()
        .get(host)
        .filter(|client| client.attest(fence).is_ok())
        .cloned()
}
/// A controlled owner for cheap connection-context tests, not a verified WSS.
pub(crate) fn pair(fence: ChannelFence, limits: BrokerLimits) -> (Client, Box<dyn Send>) {
    let (client, owner) = SocketOwner::new(fence, limits, Arc::new(|| Ok(()))).unwrap();
    (client, Box::new(owner))
}
