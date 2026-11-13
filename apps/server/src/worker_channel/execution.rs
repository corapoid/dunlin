//! Opaque live-v3 registration and bounded typed correlation. No socket, Store,
//! filesystem or executor runs while a registry mutex is held.
use super::*;
use dunlin_relay_protocol::worker::{
    ChannelFence, ExecutionControllerMessage as Request, ExecutionWorkerMessage as Reply,
};
use std::{collections::BTreeMap, sync::Mutex};
use tokio::sync::{mpsc, oneshot};
const PENDING: usize = 8;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecutionTransportError {
    Unavailable,
    Busy,
    Timeout,
    Stale,
    InvalidResponse,
}
impl std::fmt::Display for ExecutionTransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "Execution transport unavailable",
            Self::Busy => "Execution transport capacity reached",
            Self::Timeout => "Execution response deadline exceeded",
            Self::Stale => "Execution transport fence changed",
            Self::InvalidResponse => "Invalid execution request or response",
        })
    }
}
impl std::error::Error for ExecutionTransportError {}
/// Producer receives only validated frames tied to a sealed live transport.
/// It must persist/authorize them outside the socket task; this is not OS proof.
pub(crate) struct ExecutionEvent {
    pub transport: ExecutionTransport,
    pub message: Reply,
}
struct Producer {
    alive: AtomicBool,
    sender: mpsc::Sender<ExecutionEvent>,
}
pub(crate) struct ExecutionProducerRegistration {
    producer: Arc<Producer>,
}
impl Drop for ExecutionProducerRegistration {
    fn drop(&mut self) {
        self.producer.alive.store(false, Ordering::Release);
    }
}
struct Slot {
    identity: WorkerIdentity,
    lease: ChannelLease,
    fence: ChannelFence,
    controller_node_id: String,
    alive: AtomicBool,
    producer: Arc<Producer>,
    sender: mpsc::Sender<ExecutionRequest>,
    permits: Arc<Semaphore>,
    acknowledged: Mutex<BTreeMap<String, Reply>>,
}
impl Slot {
    fn live(&self) -> bool {
        self.alive.load(Ordering::Acquire)
            && self.producer.alive.load(Ordering::Acquire)
            && !self.producer.sender.is_closed()
            && !self.sender.is_closed()
    }
}
#[derive(Clone)]
pub(crate) struct ExecutionTransport {
    slot: Arc<Slot>,
}
impl ExecutionTransport {
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
            && self.slot.controller_node_id == node
            && identity.host_id() == self.identity().host_id()
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
        if !self.slot.live()
            || fence != self.fence()
            || identity.host_id() != self.host_id()
            || identity.workspace_id() != self.identity().workspace_id()
        {
            return Err(crate::store::StoreError::Conflict(
                "Execution transport replaced or unavailable".into(),
            ));
        }
        let now = now_ms().map_err(|_| {
            crate::store::StoreError::Conflict("Execution clock unavailable".into())
        })?;
        store.validate_worker_channel_in_transaction(
            self.identity(),
            self.lease(),
            &self.fence().controller_boot_id,
            now,
        )?;
        store.validate_worker_channel_in_transaction(
            identity,
            lease,
            &fence.controller_boot_id,
            now,
        )?;
        if lease.epoch != self.lease().epoch
            || lease.channel_id != self.lease().channel_id
            || lease.worker_boot_id != self.lease().worker_boot_id
        {
            return Err(crate::store::StoreError::Conflict(
                "Execution transport fence differs".into(),
            ));
        }
        if !self.slot.live() {
            return Err(crate::store::StoreError::Conflict(
                "Execution transport closed during attestation".into(),
            ));
        }
        Ok(())
    }
    /// Private evidence exists only after an exact pending ACK RPC matched a
    /// validated wire receipt. No socket-write flag or peer JSON constructor
    /// can mint it. Eviction merely requires a safe identical ACK retry.
    pub fn attest_outcome_ack_in_transaction(
        &self,
        store: &crate::store::Store,
        message: &Request,
    ) -> crate::store::Result<()> {
        let Request::OutcomeAck { channel, .. } = message else {
            return Err(crate::store::StoreError::Conflict(
                "An exact outcome ACK is required".into(),
            ));
        };
        let reply = {
            let known = self.slot.acknowledged.lock().map_err(|_| {
                crate::store::StoreError::Conflict("ACK evidence unavailable".into())
            })?;
            known.get(&request_key(message)).cloned()
        }; // No cache guard survives SQL validation.
        if !reply
            .as_ref()
            .is_some_and(|reply| matches_reply(message, reply))
        {
            return Err(crate::store::StoreError::Conflict(
                "Matching durable worker ACK receipt missing".into(),
            ));
        }
        self.attest_in_transaction(store, self.identity(), self.lease(), channel)?;
        Ok(())
    }
    fn remember_ack(&self, message: &Request, reply: &Reply) -> Result<()> {
        if !matches!(message, Request::OutcomeAck { .. })
            || !matches!(reply, Reply::OutcomeAcknowledged { .. })
            || !matches_reply(message, reply)
        {
            return Err(WorkerErrorCode::InvalidMessage);
        }
        let mut receipts = self
            .slot
            .acknowledged
            .lock()
            .map_err(|_| WorkerErrorCode::InternalError)?;
        if receipts.len() >= 32 && !receipts.contains_key(&request_key(message)) {
            receipts.pop_first();
        }
        receipts.insert(request_key(message), reply.clone());
        Ok(())
    }
    fn check(&self, message: &Request) -> std::result::Result<(), ExecutionTransportError> {
        if !self.slot.live() {
            return Err(ExecutionTransportError::Unavailable);
        }
        message
            .validate()
            .map_err(|_| ExecutionTransportError::InvalidResponse)?;
        let channel = match message {
            Request::Execute { command } => {
                let scope = &command.descriptor.scope;
                let host = self.identity().host();
                if scope.host_id != host.id
                    || scope.workspace_id != host.workspace_id
                    || scope.worker_node_id != host.definition.node_id
                    || scope.controller_node_id != self.slot.controller_node_id
                    || scope.enrollment_epoch != host.enrollment_epoch
                    || scope.credential_revision != host.credential_revision
                {
                    return Err(ExecutionTransportError::Stale);
                }
                &command.channel
            }
            Request::Renew { fence, .. } | Request::Cancel { fence } => &fence.channel,
            Request::Reconcile { channel, .. } | Request::OutcomeAck { channel, .. } => channel,
        };
        if channel != self.fence() {
            return Err(ExecutionTransportError::Stale);
        }
        Ok(())
    }
    async fn enqueue(
        &self,
        message: Request,
    ) -> std::result::Result<Option<Reply>, ExecutionTransportError> {
        self.check(&message)?;
        let permit = self
            .slot
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| ExecutionTransportError::Busy)?;
        let (response, receiver) = oneshot::channel();
        let deadline = tokio::time::Instant::now() + SEND;
        self.slot
            .sender
            .try_send(ExecutionRequest {
                message,
                deadline,
                response,
                _permit: permit,
            })
            .map_err(|_| ExecutionTransportError::Busy)?;
        tokio::time::timeout_at(deadline, receiver)
            .await
            .map_err(|_| ExecutionTransportError::Timeout)?
            .map_err(|_| ExecutionTransportError::Unavailable)?
    }
    pub async fn request(
        &self,
        message: Request,
    ) -> std::result::Result<Reply, ExecutionTransportError> {
        if matches!(message, Request::OutcomeAck { .. }) {
            return Err(ExecutionTransportError::InvalidResponse);
        }
        self.enqueue(message)
            .await?
            .ok_or(ExecutionTransportError::InvalidResponse)
    }
    pub async fn send_ack(
        &self,
        message: Request,
    ) -> std::result::Result<(), ExecutionTransportError> {
        if !matches!(message, Request::OutcomeAck { .. }) {
            return Err(ExecutionTransportError::InvalidResponse);
        }
        match self.enqueue(message).await? {
            Some(Reply::OutcomeAcknowledged { .. }) => Ok(()),
            Some(Reply::OutcomeAckRejected { .. }) => Err(ExecutionTransportError::Unavailable),
            _ => Err(ExecutionTransportError::InvalidResponse),
        }
    }
    fn publish(&self, message: Reply) -> Result<()> {
        if !self.slot.live() {
            return Err(WorkerErrorCode::ChannelExpired);
        }
        self.slot
            .producer
            .sender
            .try_send(ExecutionEvent {
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
pub(super) struct ExecutionRequest {
    message: Request,
    deadline: tokio::time::Instant,
    response: oneshot::Sender<std::result::Result<Option<Reply>, ExecutionTransportError>>,
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
    }
}
impl Registry {
    pub fn ready(&self) -> bool {
        self.producer
            .lock()
            .ok()
            .and_then(|p| p.clone())
            .is_some_and(|p| p.alive.load(Ordering::Acquire) && !p.sender.is_closed())
    }
    pub fn stop(&self) {
        if let Ok(p) = self.producer.lock()
            && let Some(p) = p.as_ref()
        {
            p.alive.store(false, Ordering::Release);
        }
        if let Ok(slots) = self.slots.lock() {
            for slot in slots.values() {
                slot.alive.store(false, Ordering::Release);
            }
        }
    }
}
impl Service {
    pub(crate) fn register_execution_producer(
        self: &Arc<Self>,
        sender: mpsc::Sender<ExecutionEvent>,
    ) -> std::result::Result<ExecutionProducerRegistration, ExecutionTransportError> {
        if self.shutdown.load(Ordering::Acquire) || sender.is_closed() || sender.max_capacity() > 32
        {
            return Err(ExecutionTransportError::Unavailable);
        }
        let mut current = self
            .executions
            .producer
            .lock()
            .map_err(|_| ExecutionTransportError::Unavailable)?;
        if current
            .as_ref()
            .is_some_and(|p| p.alive.load(Ordering::Acquire) && !p.sender.is_closed())
        {
            return Err(ExecutionTransportError::Busy);
        }
        let producer = Arc::new(Producer {
            alive: AtomicBool::new(true),
            sender,
        });
        *current = Some(producer.clone());
        Ok(ExecutionProducerRegistration { producer })
    }
    pub(crate) fn execution_transport(
        &self,
        host: &str,
    ) -> std::result::Result<ExecutionTransport, ExecutionTransportError> {
        self.executions
            .slots
            .lock()
            .map_err(|_| ExecutionTransportError::Unavailable)?
            .get(host)
            .filter(|slot| slot.live())
            .cloned()
            .map(|slot| ExecutionTransport { slot })
            .ok_or(ExecutionTransportError::Unavailable)
    }
    pub(super) fn register_execution_transport(
        &self,
        state: &AppState,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        controller_node_id: String,
    ) -> Result<(
        Registration,
        ExecutionTransport,
        mpsc::Receiver<ExecutionRequest>,
    )> {
        if !state
            .config
            .worker_controller
            .as_ref()
            .is_some_and(|config| config.execution_enabled)
        {
            return Err(WorkerErrorCode::UnsupportedProtocol);
        }
        // Always Store -> registry; no registry guard survives a Store operation.
        state
            .store
            .lock()
            .map_err(|_| WorkerErrorCode::InternalError)?
            .validate_worker_channel(identity, lease, &self.boot_id, now_ms()?)
            .map_err(|_| WorkerErrorCode::StaleLease)?;
        let producer = self
            .executions
            .producer
            .lock()
            .map_err(|_| WorkerErrorCode::InternalError)?
            .clone()
            .filter(|p| p.alive.load(Ordering::Acquire) && !p.sender.is_closed())
            .ok_or(WorkerErrorCode::UnsupportedProtocol)?;
        let (sender, receiver) = mpsc::channel(PENDING);
        let slot = Arc::new(Slot {
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
            .executions
            .slots
            .lock()
            .map_err(|_| WorkerErrorCode::InternalError)?;
        slots.retain(|_, current| current.live());
        if slots.len() >= SLOTS && !slots.contains_key(identity.host_id()) {
            return Err(WorkerErrorCode::CapacityReached);
        }
        if let Some(previous) = slots.get(identity.host_id()) {
            if previous.lease.epoch >= lease.epoch {
                return Err(WorkerErrorCode::StaleLease);
            }
            previous.alive.store(false, Ordering::Release);
        }
        slots.insert(identity.host_id().into(), slot.clone());
        Ok((
            Registration { slot: slot.clone() },
            ExecutionTransport { slot },
            receiver,
        ))
    }
}
fn request_key(message: &Request) -> String {
    match message {
        Request::OutcomeAck {
            identity,
            outcome_id,
            outcome_digest,
            ..
        } => format!("ack:{}:{outcome_id}:{outcome_digest}", identity.attempt_id),
        Request::Execute { command } => format!("request:{}", command.identity.attempt_id),
        Request::Renew { fence, .. } | Request::Cancel { fence } => {
            format!("request:{}", fence.attempt_id)
        }
        Request::Reconcile { identity, .. } => format!("request:{}", identity.attempt_id),
    }
}
fn reply_key(reply: &Reply) -> String {
    match reply {
        Reply::OutcomeAcknowledged {
            identity,
            outcome_id,
            outcome_digest,
            ..
        }
        | Reply::OutcomeAckRejected {
            identity,
            outcome_id,
            outcome_digest,
            ..
        } => format!("ack:{}:{outcome_id}:{outcome_digest}", identity.attempt_id),
        Reply::Accepted { record } => format!("request:{}", record.receipt.identity.attempt_id),
        Reply::Renewed { fence, .. }
        | Reply::CancelAck { fence, .. }
        | Reply::Outcome { fence, .. }
        | Reply::ControlRejected { fence, .. } => format!("request:{}", fence.attempt_id),
        Reply::Reconciled { identity, .. } | Reply::Rejected { identity, .. } => {
            format!("request:{}", identity.attempt_id)
        }
    }
}
fn matches_reply(request: &Request, reply: &Reply) -> bool {
    match (request, reply) {
        (Request::Execute { command }, Reply::Accepted { record }) => {
            record.receipt.identity == command.identity
                && record.receipt.fence.channel == command.channel
                && record.receipt.lease_duration_ms == command.budgets.lease_duration_ms
        }
        (
            Request::Execute { command },
            Reply::Rejected {
                channel, identity, ..
            },
        ) => channel == &command.channel && identity == &command.identity,
        (
            Request::Renew {
                fence,
                sequence,
                lease_duration_ms,
            },
            Reply::Renewed {
                fence: actual,
                sequence: seq,
                lease_duration_ms: ttl,
            },
        ) => fence == actual && sequence == seq && lease_duration_ms == ttl,
        (
            Request::Renew { fence, .. } | Request::Cancel { fence },
            Reply::ControlRejected { fence: actual, .. },
        ) => fence == actual,
        (Request::Cancel { fence }, Reply::CancelAck { fence: actual, .. }) => fence == actual,
        (
            Request::Reconcile { channel, identity },
            Reply::Reconciled {
                channel: actual,
                identity: actual_id,
                ..
            }
            | Reply::Rejected {
                channel: actual,
                identity: actual_id,
                ..
            },
        ) => channel == actual && identity == actual_id,
        (
            Request::OutcomeAck {
                channel,
                identity,
                fence,
                outcome_id,
                outcome_digest,
            },
            Reply::OutcomeAcknowledged {
                channel: actual,
                identity: actual_id,
                fence: actual_fence,
                outcome_id: actual_outcome,
                outcome_digest: actual_digest,
            }
            | Reply::OutcomeAckRejected {
                channel: actual,
                identity: actual_id,
                fence: actual_fence,
                outcome_id: actual_outcome,
                outcome_digest: actual_digest,
                ..
            },
        ) => {
            channel == actual
                && identity == actual_id
                && fence == actual_fence
                && outcome_id == actual_outcome
                && outcome_digest == actual_digest
        }
        _ => false,
    }
}
/// Accepted receipts may also be repeated by the durable outbox, and Outcome
/// is always an unsolicited event. Neither consumes another RPC's waiter.
/// An Accepted frame aimed at a pending Execute remains strictly correlated:
/// mismatched identity, channel or lease is a protocol error, not an event.
fn take_pending_reply(
    pending: &mut BTreeMap<String, ExecutionRequest>,
    reply: &Reply,
) -> Result<Option<ExecutionRequest>> {
    let key = reply_key(reply);
    let Some(request) = pending.get(&key) else {
        return Ok(None);
    };
    if matches!(reply, Reply::Outcome { .. })
        || (matches!(reply, Reply::Accepted { .. })
            && !matches!(request.message, Request::Execute { .. }))
    {
        return Ok(None);
    }
    if !matches_reply(&request.message, reply) {
        return Err(WorkerErrorCode::InvalidMessage);
    }
    Ok(pending.remove(&key))
}
async fn send_execution(
    socket: &mut WebSocket,
    message: &Request,
    deadline: tokio::time::Instant,
) -> Result<()> {
    if tokio::time::Instant::now() >= deadline {
        return Err(WorkerErrorCode::ChannelExpired);
    }
    message
        .validate()
        .map_err(|_| WorkerErrorCode::InvalidMessage)?;
    let text = serde_json::to_string(message).map_err(|_| WorkerErrorCode::InternalError)?;
    tokio::time::timeout_at(
        deadline.min(tokio::time::Instant::now() + SEND),
        socket.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| WorkerErrorCode::ChannelExpired)?
    .map_err(|_| WorkerErrorCode::ChannelExpired)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn messages(
    socket: &mut WebSocket,
    state: &AppState,
    identity: &WorkerIdentity,
    lease: &mut ChannelLease,
    rate: &mut Rate,
    metadata: &mut mpsc::Receiver<exports::DescribeRequest>,
    mut requests: mpsc::Receiver<ExecutionRequest>,
    transport: ExecutionTransport,
) -> Result<()> {
    let exchange = async {
        let mut pending: BTreeMap<String, ExecutionRequest> = BTreeMap::new();
        let mut descriptions: BTreeMap<String, exports::DescribeRequest> = BTreeMap::new();
        let mut heartbeats = tokio::task::JoinSet::new();
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                _=tick.tick()=>{
                    let now=tokio::time::Instant::now();
                    pending.retain(|_,r|now<r.deadline&&!r.response.is_closed());
                    descriptions.retain(|_,r|now<r.deadline&&!r.response.is_closed());
                },
                request=requests.recv()=>{
                    let Some(request)=request else{return Err(WorkerErrorCode::ChannelExpired)};
                    if request.response.is_closed()||tokio::time::Instant::now()>=request.deadline{continue;}
                    if transport.check(&request.message).is_err(){let _=request.response.send(Err(ExecutionTransportError::Stale));continue;}
                    let key=request_key(&request.message);
                    if pending.contains_key(&key){let _=request.response.send(Err(ExecutionTransportError::Busy));continue;}
                    let message=request.message.clone();let deadline=request.deadline;pending.insert(key,request);
                    send_execution(socket,&message,deadline).await?;
                },
                request=metadata.recv()=>{
                    let Some(request)=request else{return Err(WorkerErrorCode::ChannelExpired)};
                    if request.response.is_closed()||tokio::time::Instant::now()>=request.deadline{continue;}
                    let message=ControllerMessage::DescribeExport{request_id:request.request_id.clone(),channel_id:lease.channel_id.clone(),lease_epoch:lease.epoch,
                        export_id:request.export_id.clone(),principal:request.principal.clone(),expected_export_revision:request.expected};
                    descriptions.insert(request.request_id.clone(),request);send(socket,message).await?;
                },
                result=heartbeats.join_next(),if !heartbeats.is_empty()=>{
                    let (renewed, received)=result.ok_or(WorkerErrorCode::InternalError)?.map_err(|_|WorkerErrorCode::InternalError)??;
                    *lease = renewed;
                    state.worker_channels.observed_heartbeat(lease, received);
                    send(socket,ControllerMessage::HeartbeatAck{channel_id:lease.channel_id.clone(),lease_epoch:lease.epoch,sequence:lease.last_sequence,
                        expires_at_ms:u64::try_from(lease.expires_at_ms).map_err(|_|WorkerErrorCode::InternalError)?}).await?;
                },
                frame=socket.recv()=>{
                    let frame=frame.ok_or(WorkerErrorCode::ChannelExpired)?.map_err(|_|WorkerErrorCode::InvalidMessage)?;
                    rate.frame_for_protocol(WORKER_EXECUTION_PROTOCOL_VERSION)?;
                    let Message::Text(text)=frame else {
                        if matches!(frame,Message::Ping(_)|Message::Pong(_)){continue;}
                        return Err(WorkerErrorCode::ChannelExpired)
                    };
                    if let Ok(message)=worker::decode_frame(&text) {
                        match message {
                            WorkerMessage::Heartbeat{channel_id,lease_epoch,sequence}=>{
                                if channel_id!=lease.channel_id||lease_epoch!=lease.epoch||!heartbeats.is_empty(){return Err(WorkerErrorCode::InvalidSequence)}
                                let store=state.store.clone();let identity=identity.clone();let lease=lease.clone();
                                let received = std::time::Instant::now();
                                heartbeats.spawn_blocking(move|| {
                                    let store=store.lock().map_err(|_|WorkerErrorCode::InternalError)?;
                                    store.renew_worker_channel(&identity,&lease,sequence,now_ms()?,TTL_MS).map_err(|_|WorkerErrorCode::InvalidSequence).map(|lease| (lease, received))
                                });
                            },
                            WorkerMessage::ExportDescribed{request_id,channel_id,lease_epoch,descriptor}=>{
                                if channel_id!=lease.channel_id||lease_epoch!=lease.epoch{return Err(WorkerErrorCode::StaleLease)}
                                if let Some(request)=descriptions.remove(&request_id)&&tokio::time::Instant::now()<request.deadline {
                                    let _=request.response.send(Ok(descriptor));
                                }
                            },
                            WorkerMessage::ExportError{request_id,channel_id,lease_epoch,code}=>{
                                if channel_id!=lease.channel_id||lease_epoch!=lease.epoch{return Err(WorkerErrorCode::StaleLease)}
                                if let Some(request)=descriptions.remove(&request_id){
                                    let code=match code {worker::ExportErrorCode::Unavailable=>DescribeError::Unavailable,worker::ExportErrorCode::Forbidden=>DescribeError::Forbidden,
                                        worker::ExportErrorCode::StaleExport=>DescribeError::StaleExport,worker::ExportErrorCode::Busy=>DescribeError::Busy};
                                    let _=request.response.send(Err(code));
                                }
                            },
                            _=>return Err(WorkerErrorCode::InvalidMessage),
                        }
                        continue;
                    }
                    let reply=worker::decode_execution_worker_frame(&text).map_err(|_|WorkerErrorCode::InvalidMessage)?;
                    let fence=match &reply {
                        Reply::Accepted{record}=>&record.receipt.fence.channel,
                        Reply::Renewed{fence,..}|Reply::CancelAck{fence,..}|Reply::Outcome{fence,..}|Reply::ControlRejected{fence,..}=>&fence.channel,
                        Reply::Reconciled{channel,..}|Reply::Rejected{channel,..}|Reply::OutcomeAcknowledged{channel,..}|Reply::OutcomeAckRejected{channel,..}=>channel,
                    };
                    if fence!=transport.fence(){return Err(WorkerErrorCode::StaleLease)}
                    if let Some(request)=take_pending_reply(&mut pending,&reply)? {
                        if matches!(reply,Reply::OutcomeAcknowledged{..}){transport.remember_ack(&request.message,&reply)?;}
                        if tokio::time::Instant::now()<request.deadline&&!request.response.is_closed(){let _=request.response.send(Ok(Some(reply)));}
                        else if matches!(reply,Reply::Accepted{..}|Reply::Outcome{..}){transport.publish(reply)?;}
                    } else if matches!(reply,Reply::Accepted{..}|Reply::Outcome{..}) {
                        transport.publish(reply)?;
                    } else if matches!(reply,Reply::Reconciled{..}|Reply::Rejected{..}|Reply::ControlRejected{..}|Reply::OutcomeAcknowledged{..}|Reply::OutcomeAckRejected{..}) {
                        // A late query/error is never interpreted as another
                        // request's success or as an unsolicited positive action.
                    }
                },
            }
        }
    };
    tokio::select! {result=exchange=>result,_=transport.stopped()=>Err(WorkerErrorCode::ChannelExpired)}
}
#[cfg(test)]
mod tests;
