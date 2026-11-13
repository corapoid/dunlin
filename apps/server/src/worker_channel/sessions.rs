//! Opaque v4/v5 transport. Registration follows verified WSS; every positive
//! consumer still needs fresh SQL policy and an exact live-channel attestation.
use super::execution::ExecutionTransportError as TransportError;
use super::*;
use dunlin_relay_protocol::worker::{
    ChannelFence,
    session::{self as wire, SessionControllerMessage as Request, SessionWorkerMessage as Reply},
};
use std::{collections::BTreeMap, sync::Mutex};
use tokio::sync::{mpsc, oneshot};

mod control;
use control::Control;

const PENDING: usize = 8;
pub(crate) struct SessionEvent {
    pub transport: SessionTransport,
    pub message: Reply,
}
struct Producer {
    alive: AtomicBool,
    sender: mpsc::Sender<SessionEvent>,
}
pub(crate) struct SessionProducerRegistration {
    producer: Arc<Producer>,
}
impl Drop for SessionProducerRegistration {
    fn drop(&mut self) {
        self.producer.alive.store(false, Ordering::Release);
    }
}
struct Slot {
    brokers: Arc<broker::Registry>,
    broker_offer: Option<wire::broker::BrokerLimits>,
    identity: WorkerIdentity,
    lease: ChannelLease,
    fence: ChannelFence,
    controller_node_id: String,
    alive: AtomicBool,
    producer: Arc<Producer>,
    sender: mpsc::Sender<SessionRequest>,
    permits: Arc<Semaphore>,
    acknowledged: Mutex<BTreeMap<String, Reply>>,
}
impl Slot {
    fn live(&self) -> bool {
        self.alive.load(Ordering::Acquire)
            && self.producer.alive.load(Ordering::Acquire)
            && !self.sender.is_closed()
            && !self.producer.sender.is_closed()
    }
}
#[derive(Clone)]
pub(crate) struct SessionTransport {
    slot: Arc<Slot>,
}
impl SessionTransport {
    pub(crate) fn bind_broker(
        &self,
        handler: Arc<dyn super::SessionBroker>,
    ) -> Result<super::BrokerRegistration> {
        if self.broker_offer().is_none() || !self.slot.live() {
            return Err(WorkerErrorCode::UnsupportedProtocol);
        }
        self.slot.brokers.bind(handler)
    }
    pub(super) fn is_live(&self) -> bool {
        self.slot.live()
    }
    pub(super) fn broker_exchange(&self) -> broker::Exchange {
        broker::Exchange::new(self.clone(), self.slot.brokers.clone())
    }
    /// Controlled in-process transport driver; no verified WSS is fabricated.
    #[cfg(test)]
    pub(crate) async fn broker_request_fixture(
        &self,
        request: wire::broker::BrokerRequest,
    ) -> std::result::Result<String, wire::broker::BrokerError> {
        let mut exchange = self.broker_exchange();
        let mut assembly = wire::broker::ReplyAssembly::new(&request).unwrap();
        let mut immediate = exchange
            .submit(request)
            .map_err(|_| wire::broker::BrokerError::Unavailable)?;
        loop {
            let reply = match immediate.take() {
                Some(reply) => reply,
                None => exchange
                    .next()
                    .await
                    .map_err(|_| wire::broker::BrokerError::Unavailable)?,
            };
            match reply {
                wire::broker::BrokerControllerMessage::McpRejected { code, .. } => {
                    return Err(code);
                }
                wire::broker::BrokerControllerMessage::McpReplyChunk { chunk } => {
                    if let Some(value) = assembly.push(&chunk).unwrap() {
                        return Ok(value);
                    }
                }
            }
        }
    }
    #[cfg(test)]
    pub(crate) fn broker_open_fixture(&self) -> wire::broker::OpenBrokerSessionCommand {
        tests::broker_command(self)
    }
    pub(crate) fn broker_offer(&self) -> Option<&wire::broker::BrokerLimits> {
        self.slot.broker_offer.as_ref()
    }
    pub fn host_id(&self) -> &str {
        self.slot.identity.host_id()
    }
    pub fn identity(&self) -> &WorkerIdentity {
        &self.slot.identity
    }
    pub fn lease(&self) -> &ChannelLease {
        &self.slot.lease
    }
    pub fn fence(&self) -> &ChannelFence {
        &self.slot.fence
    }
    pub(super) fn matches_metadata_slot(
        &self,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        node: &str,
    ) -> bool {
        self.slot.live()
            && node == self.slot.controller_node_id
            && identity.host_id() == self.host_id()
            && identity.workspace_id() == self.identity().workspace_id()
            && identity.host().enrollment_epoch == self.identity().host().enrollment_epoch
            && identity.host().credential_revision == self.identity().host().credential_revision
            && identity.host().definition.node_id == self.identity().host().definition.node_id
            && lease.host_id == self.lease().host_id
            && lease.workspace_id == self.lease().workspace_id
            && lease.epoch == self.lease().epoch
            && lease.channel_id == self.fence().channel_id
            && lease.controller_boot_id == self.fence().controller_boot_id
            && lease.worker_boot_id == self.fence().worker_boot_id
    }
    pub fn attest_in_transaction(
        &self,
        store: &crate::store::Store,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        fence: &ChannelFence,
    ) -> crate::store::Result<()> {
        let denied = || {
            crate::store::StoreError::Conflict("Session transport changed or unavailable".into())
        };
        if !self.matches_metadata_slot(identity, lease, &self.slot.controller_node_id)
            || fence != self.fence()
        {
            return Err(denied());
        }
        let now = now_ms().map_err(|_| denied())?;
        store.validate_worker_channel_in_transaction(
            self.identity(),
            self.lease(),
            &fence.controller_boot_id,
            now,
        )?;
        store.validate_worker_channel_in_transaction(
            identity,
            lease,
            &fence.controller_boot_id,
            now,
        )?;
        if !self.slot.live() {
            return Err(denied());
        }
        Ok(())
    }
    fn check(&self, request: &Control) -> std::result::Result<(), TransportError> {
        if !self.slot.live() {
            return Err(TransportError::Unavailable);
        }
        request
            .validate_for_channel(self.fence(), self.broker_offer())
            .map_err(|_| TransportError::InvalidResponse)?;
        if let Some(command) = request.open() {
            let scope = &command.descriptor.scope;
            let host = self.identity().host();
            if scope.host_id != host.id
                || scope.workspace_id != host.workspace_id
                || scope.worker_node_id != host.definition.node_id
                || scope.controller_node_id != self.slot.controller_node_id
                || scope.enrollment_epoch != host.enrollment_epoch
                || scope.credential_revision != host.credential_revision
            {
                return Err(TransportError::Stale);
            }
        }
        Ok(())
    }
    pub async fn request(&self, message: Request) -> std::result::Result<Reply, TransportError> {
        self.send_control(Control::Legacy(Box::new(message))).await
    }
    pub(crate) async fn open_broker_session(
        &self,
        command: wire::broker::OpenBrokerSessionCommand,
    ) -> std::result::Result<Reply, TransportError> {
        self.send_control(Control::Broker(Box::new(command))).await
    }
    async fn send_control(&self, message: Control) -> std::result::Result<Reply, TransportError> {
        self.check(&message)?;
        let permit = self
            .slot
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| TransportError::Busy)?;
        let (response, receiver) = oneshot::channel();
        let deadline = tokio::time::Instant::now() + SEND;
        self.slot
            .sender
            .try_send(SessionRequest {
                message,
                deadline,
                response,
                _permit: permit,
            })
            .map_err(|_| TransportError::Busy)?;
        tokio::time::timeout_at(deadline, receiver)
            .await
            .map_err(|_| TransportError::Timeout)?
            .map_err(|_| TransportError::Unavailable)?
    }
    pub fn attest_stream_ack_in_transaction(
        &self,
        store: &crate::store::Store,
        request: &Request,
    ) -> crate::store::Result<()> {
        let denied = || {
            crate::store::StoreError::Conflict("Exact durable session stream ACK missing".into())
        };
        if !matches!(request, Request::AckStream { .. }) {
            return Err(denied());
        }
        let key = request_key(request).map_err(|_| denied())?;
        let receipt = self
            .slot
            .acknowledged
            .lock()
            .map_err(|_| denied())?
            .get(&key)
            .cloned();
        if !receipt.as_ref().is_some_and(|reply| {
            matches!(reply, Reply::StreamAcknowledged { .. })
                && reply.validate_for_request(request).is_ok()
        }) {
            return Err(denied());
        }
        self.attest_in_transaction(store, self.identity(), self.lease(), self.fence())
    }
    fn publish(&self, message: Reply) -> Result<()> {
        if !self.slot.live() {
            return Err(WorkerErrorCode::ChannelExpired);
        }
        self.slot
            .producer
            .sender
            .try_send(SessionEvent {
                transport: self.clone(),
                message,
            })
            .map_err(|_| WorkerErrorCode::CapacityReached)
    }
    async fn stopped(&self) {
        while self.slot.live() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
pub(super) struct SessionRequest {
    message: Control,
    deadline: tokio::time::Instant,
    response: oneshot::Sender<std::result::Result<Reply, TransportError>>,
    _permit: OwnedSemaphorePermit,
}
#[derive(Default)]
pub(super) struct Registry {
    producer: Mutex<Option<Arc<Producer>>>,
    slots: Mutex<BTreeMap<String, Arc<Slot>>>,
}
pub(super) struct Registration {
    slot: Arc<Slot>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.slot.alive.store(false, Ordering::Release);
        self.slot.brokers.stop();
    }
}
impl Registry {
    pub fn ready(&self) -> bool {
        self.producer
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .is_some_and(|p| p.alive.load(Ordering::Acquire) && !p.sender.is_closed())
    }
    pub fn stop(&self) {
        if let Ok(producer) = self.producer.lock()
            && let Some(producer) = producer.as_ref()
        {
            producer.alive.store(false, Ordering::Release);
        }
        if let Ok(slots) = self.slots.lock() {
            for slot in slots.values() {
                slot.alive.store(false, Ordering::Release);
                slot.brokers.stop();
            }
        }
    }
}
impl Service {
    pub(crate) fn register_session_producer(
        self: &Arc<Self>,
        sender: mpsc::Sender<SessionEvent>,
    ) -> std::result::Result<SessionProducerRegistration, TransportError> {
        if self.shutdown.load(Ordering::Acquire) || sender.is_closed() || sender.max_capacity() > 32
        {
            return Err(TransportError::Unavailable);
        }
        let mut current = self
            .sessions
            .producer
            .lock()
            .map_err(|_| TransportError::Unavailable)?;
        if current
            .as_ref()
            .is_some_and(|p| p.alive.load(Ordering::Acquire) && !p.sender.is_closed())
        {
            return Err(TransportError::Busy);
        }
        let producer = Arc::new(Producer {
            alive: AtomicBool::new(true),
            sender,
        });
        *current = Some(producer.clone());
        Ok(SessionProducerRegistration { producer })
    }
    pub(crate) fn session_transport(
        &self,
        host: &str,
    ) -> std::result::Result<SessionTransport, TransportError> {
        self.sessions
            .slots
            .lock()
            .map_err(|_| TransportError::Unavailable)?
            .get(host)
            .filter(|slot| slot.live())
            .cloned()
            .map(|slot| SessionTransport { slot })
            .ok_or(TransportError::Unavailable)
    }
    pub(super) fn register_session_transport(
        &self,
        state: &AppState,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        controller_node_id: String,
    ) -> Result<(
        Registration,
        SessionTransport,
        mpsc::Receiver<SessionRequest>,
    )> {
        self.register_transport(state, identity, lease, controller_node_id, None)
    }
    pub(super) fn register_broker_session_transport(
        &self,
        state: &AppState,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        controller_node_id: String,
        offer: wire::broker::BrokerLimits,
    ) -> Result<(
        Registration,
        SessionTransport,
        mpsc::Receiver<SessionRequest>,
    )> {
        offer
            .validate()
            .map_err(|_| WorkerErrorCode::InvalidMessage)?;
        if !state
            .config
            .worker_controller
            .as_ref()
            .is_some_and(|config| config.mcp_broker)
        {
            return Err(WorkerErrorCode::UnsupportedProtocol);
        }
        self.register_transport(state, identity, lease, controller_node_id, Some(offer))
    }
    fn register_transport(
        &self,
        state: &AppState,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        controller_node_id: String,
        broker_offer: Option<wire::broker::BrokerLimits>,
    ) -> Result<(
        Registration,
        SessionTransport,
        mpsc::Receiver<SessionRequest>,
    )> {
        // The separate producer must be present; v3 execution opt-in alone is
        // never permission to reinterpret one-turn execution as a live session.
        if !state
            .config
            .worker_controller
            .as_ref()
            .is_some_and(|config| config.execution_enabled && config.persistent_sessions)
        {
            return Err(WorkerErrorCode::UnsupportedProtocol);
        }
        state
            .store
            .lock()
            .map_err(|_| WorkerErrorCode::InternalError)?
            .validate_worker_channel(identity, lease, &self.boot_id, now_ms()?)
            .map_err(|_| WorkerErrorCode::StaleLease)?;
        let producer = self
            .sessions
            .producer
            .lock()
            .map_err(|_| WorkerErrorCode::InternalError)?
            .clone()
            .filter(|p| p.alive.load(Ordering::Acquire) && !p.sender.is_closed())
            .ok_or(WorkerErrorCode::UnsupportedProtocol)?;
        let (sender, receiver) = mpsc::channel(PENDING);
        let slot = Arc::new(Slot {
            brokers: Arc::new(broker::Registry::default()),
            broker_offer,
            identity: identity.clone(),
            lease: lease.clone(),
            fence: ChannelFence {
                channel_id: lease.channel_id.clone(),
                controller_boot_id: lease.controller_boot_id.clone(),
                worker_boot_id: lease.worker_boot_id.clone(),
            },
            controller_node_id,
            alive: AtomicBool::new(true),
            producer,
            sender,
            permits: Arc::new(Semaphore::new(PENDING)),
            acknowledged: Mutex::new(BTreeMap::new()),
        });
        let mut slots = self
            .sessions
            .slots
            .lock()
            .map_err(|_| WorkerErrorCode::InternalError)?;
        slots.retain(|_, previous| previous.live());
        if slots.len() >= SLOTS && !slots.contains_key(identity.host_id()) {
            return Err(WorkerErrorCode::CapacityReached);
        }
        if let Some(previous) = slots.get(identity.host_id()) {
            if previous.lease.epoch >= lease.epoch {
                return Err(WorkerErrorCode::StaleLease);
            }
            previous.alive.store(false, Ordering::Release);
            previous.brokers.stop();
        }
        slots.insert(identity.host_id().into(), slot.clone());
        Ok((
            Registration { slot: slot.clone() },
            SessionTransport { slot },
            receiver,
        ))
    }
}
fn request_key(request: &Request) -> Result<String> {
    serde_json::to_string(&(request.channel(), request.reference()))
        .map_err(|_| WorkerErrorCode::InvalidMessage)
}
fn take_reply(
    pending: &mut BTreeMap<String, SessionRequest>,
    reply: &Reply,
) -> Result<Option<SessionRequest>> {
    // At most eight exact candidates. Neither stream events nor readiness can
    // consume a control RPC, even if their session identity happens to match.
    if matches!(reply, Reply::Event { .. } | Reply::SessionReady { .. }) {
        return Ok(None);
    }
    let key = pending
        .iter()
        .find(|(_, request)| request.message.accepts(reply))
        .map(|(key, _)| key.clone());
    Ok(key.and_then(|key| pending.remove(&key)))
}

async fn send_session(
    socket: &mut WebSocket,
    message: &Control,
    deadline: tokio::time::Instant,
) -> Result<()> {
    if tokio::time::Instant::now() >= deadline {
        return Err(WorkerErrorCode::ChannelExpired);
    }
    let text = message
        .encode()
        .map_err(|_| WorkerErrorCode::InvalidMessage)?;
    tokio::time::timeout_at(
        deadline.min(tokio::time::Instant::now() + SEND),
        socket.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| WorkerErrorCode::ChannelExpired)?
    .map_err(|_| WorkerErrorCode::ChannelExpired)
}
#[cfg(test)]
mod tests;

#[allow(clippy::too_many_arguments)]
pub(super) async fn messages(
    socket: &mut WebSocket,
    state: &AppState,
    identity: &WorkerIdentity,
    lease: &mut ChannelLease,
    rate: &mut Rate,
    metadata: &mut mpsc::Receiver<exports::DescribeRequest>,
    mut requests: mpsc::Receiver<SessionRequest>,
    transport: SessionTransport,
) -> Result<()> {
    let exchange = async {
        let mut brokers = transport.broker_exchange();
        let mut pending: BTreeMap<String, SessionRequest> = BTreeMap::new();
        let mut descriptions: BTreeMap<String, exports::DescribeRequest> = BTreeMap::new();
        let mut heartbeats = tokio::task::JoinSet::new();
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    let now = tokio::time::Instant::now();
                    pending.retain(|_, request| now < request.deadline && !request.response.is_closed());
                    descriptions.retain(|_, request| now < request.deadline && !request.response.is_closed());
                    brokers.expire()?;
                },
                reply = brokers.next(), if brokers.active() => {
                    broker::send_reply(socket, &reply?).await?;
                },
                request = requests.recv() => {
                    let Some(request) = request else { return Err(WorkerErrorCode::ChannelExpired); };
                    if request.response.is_closed() || tokio::time::Instant::now() >= request.deadline { continue; }
                    if transport.check(&request.message).is_err() { let _ = request.response.send(Err(TransportError::Stale)); continue; }
                    let key = request.message.key()?;
                    if pending.contains_key(&key) { let _ = request.response.send(Err(TransportError::Busy)); continue; }
                    let message = request.message.clone(); let deadline = request.deadline;
                    pending.insert(key, request);
                    send_session(socket, &message, deadline).await?;
                },
                request = metadata.recv() => {
                    let Some(request) = request else { return Err(WorkerErrorCode::ChannelExpired); };
                    if request.response.is_closed() || tokio::time::Instant::now() >= request.deadline { continue; }
                    let message = ControllerMessage::DescribeExport {
                        request_id: request.request_id.clone(), channel_id: lease.channel_id.clone(), lease_epoch: lease.epoch,
                        export_id: request.export_id.clone(), principal: request.principal.clone(), expected_export_revision: request.expected,
                    };
                    descriptions.insert(request.request_id.clone(), request);
                    send(socket, message).await?;
                },
                result = heartbeats.join_next(), if !heartbeats.is_empty() => {
                    let (renewed, received) = result.ok_or(WorkerErrorCode::InternalError)?.map_err(|_| WorkerErrorCode::InternalError)??;
                    *lease = renewed;
                    state.worker_channels.observed_heartbeat(lease, received);
                    send(socket, ControllerMessage::HeartbeatAck {
                        channel_id: lease.channel_id.clone(), lease_epoch: lease.epoch, sequence: lease.last_sequence,
                        expires_at_ms: u64::try_from(lease.expires_at_ms).map_err(|_| WorkerErrorCode::InternalError)?,
                    }).await?;
                },
                frame = socket.recv() => {
                    let frame = frame.ok_or(WorkerErrorCode::ChannelExpired)?.map_err(|_| WorkerErrorCode::InvalidMessage)?;
                    rate.frame_for_protocol(if transport.broker_offer().is_some() { wire::broker::WORKER_BROKER_PROTOCOL_VERSION } else { wire::WORKER_SESSION_PROTOCOL_VERSION })?;
                    let Message::Text(text) = frame else {
                        if matches!(frame, Message::Ping(_) | Message::Pong(_)) { continue; }
                        return Err(WorkerErrorCode::ChannelExpired);
                    };
                    if let Ok(message) = worker::decode_frame(&text) {
                        match message {
                            WorkerMessage::Heartbeat { channel_id, lease_epoch, sequence } => {
                                if channel_id != lease.channel_id || lease_epoch != lease.epoch || !heartbeats.is_empty() { return Err(WorkerErrorCode::InvalidSequence); }
                                let store = state.store.clone(); let identity = identity.clone(); let lease = lease.clone();
                                let received = std::time::Instant::now();
                                heartbeats.spawn_blocking(move || {
                                    store.lock().map_err(|_| WorkerErrorCode::InternalError)?
                                        .renew_worker_channel(&identity, &lease, sequence, now_ms()?, TTL_MS).map_err(|_| WorkerErrorCode::InvalidSequence).map(|lease| (lease, received))
                                });
                            },
                            WorkerMessage::ExportDescribed { request_id, channel_id, lease_epoch, descriptor } => {
                                if channel_id != lease.channel_id || lease_epoch != lease.epoch { return Err(WorkerErrorCode::StaleLease); }
                                if let Some(request) = descriptions.remove(&request_id) && tokio::time::Instant::now() < request.deadline {
                                    let _ = request.response.send(Ok(descriptor));
                                }
                            },
                            WorkerMessage::ExportError { request_id, channel_id, lease_epoch, code } => {
                                if channel_id != lease.channel_id || lease_epoch != lease.epoch { return Err(WorkerErrorCode::StaleLease); }
                                if let Some(request) = descriptions.remove(&request_id) {
                                    let code = match code {
                                        worker::ExportErrorCode::Unavailable => DescribeError::Unavailable,
                                        worker::ExportErrorCode::Forbidden => DescribeError::Forbidden,
                                        worker::ExportErrorCode::StaleExport => DescribeError::StaleExport,
                                        worker::ExportErrorCode::Busy => DescribeError::Busy,
                                    };
                                    let _ = request.response.send(Err(code));
                                }
                            },
                            _ => return Err(WorkerErrorCode::InvalidMessage),
                        }
                        continue;
                    }
                    if let Some(request) = broker::request(&text, &transport)? {
                        if let Some(reply) = brokers.submit(request)? {
                            broker::send_reply(socket, &reply).await?;
                        }
                        continue;
                    }
                    let reply = wire::decode_session_worker_frame(&text).map_err(|_| WorkerErrorCode::InvalidMessage)?;
                    reply.validate_for_channel(transport.fence()).map_err(|_| WorkerErrorCode::StaleLease)?;
                    if let Some(request) = take_reply(&mut pending, &reply)? {
                        if matches!(reply, Reply::StreamAcknowledged { .. }) {
                            let mut receipts = transport.slot.acknowledged.lock().map_err(|_| WorkerErrorCode::InternalError)?;
                            let key = request.message.key()?;
                            if receipts.len() >= 32 && !receipts.contains_key(&key) { receipts.pop_first(); }
                            receipts.insert(key, reply.clone());
                        }
                        if tokio::time::Instant::now() < request.deadline && !request.response.is_closed() {
                            let _ = request.response.send(Ok(reply));
                        } else if matches!(reply, Reply::SessionAccepted { .. } | Reply::TurnAccepted { .. }) {
                            transport.publish(reply)?;
                        }
                    } else if matches!(reply, Reply::Event { .. } | Reply::SessionReady { .. } | Reply::SessionAccepted { .. } | Reply::TurnAccepted { .. }) {
                        transport.publish(reply)?;
                    }
                },
            }
        }
    };
    tokio::select! { result = exchange => result, _ = transport.stopped() => Err(WorkerErrorCode::ChannelExpired) }
}
