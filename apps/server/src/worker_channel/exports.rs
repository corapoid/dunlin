//! Bounded request correlation on a proved v2 channel. The sealed observation
//! carries transport fences; it is not a grant to execute or a distributed lock.
use super::*;
use dunlin_relay_protocol::worker::{ExportDescriptor, ExportErrorCode, ExportPrincipal};
use std::{collections::BTreeMap, sync::Mutex, time::Instant};
use tokio::sync::{mpsc, oneshot};
const REQUEST_TTL: Duration = Duration::from_secs(5);
const PENDING: usize = 8;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DescribeError {
    Unavailable,
    Forbidden,
    StaleExport,
    Busy,
    InvalidResponse,
}
pub(crate) struct ObservedExport {
    identity: WorkerIdentity,
    lease: ChannelLease,
    descriptor: ExportDescriptor,
    observed_at_ms: i64,
    observed: Instant,
}
impl ObservedExport {
    pub fn identity(&self) -> &WorkerIdentity {
        &self.identity
    }
    pub fn lease(&self) -> &ChannelLease {
        &self.lease
    }
    pub fn descriptor(&self) -> &ExportDescriptor {
        &self.descriptor
    }
    pub fn observed_at_ms(&self) -> i64 {
        self.observed_at_ms
    }
    pub fn is_fresh(&self) -> bool {
        self.observed.elapsed() <= REQUEST_TTL
    }
    #[cfg(test)]
    pub(crate) fn for_test(
        identity: WorkerIdentity,
        lease: ChannelLease,
        descriptor: ExportDescriptor,
        observed_at_ms: i64,
    ) -> Self {
        let age = now_ms()
            .ok()
            .and_then(|now| now.checked_sub(observed_at_ms))
            .and_then(|age| u64::try_from(age).ok())
            .unwrap_or(6000);
        Self {
            identity,
            lease,
            descriptor,
            observed_at_ms,
            observed: Instant::now()
                .checked_sub(Duration::from_millis(age))
                .unwrap_or_else(Instant::now),
        }
    }
}
pub(super) struct DescribeRequest {
    pub request_id: String,
    pub export_id: String,
    pub principal: ExportPrincipal,
    pub expected: Option<u64>,
    pub deadline: tokio::time::Instant,
    pub response: oneshot::Sender<std::result::Result<ExportDescriptor, DescribeError>>,
    _permit: OwnedSemaphorePermit,
}
#[derive(Clone)]
struct Slot {
    identity: WorkerIdentity,
    lease: ChannelLease,
    controller_node_id: String,
    sender: mpsc::Sender<DescribeRequest>,
    permits: Arc<Semaphore>,
}
#[derive(Default)]
pub(super) struct Registry {
    slots: Mutex<BTreeMap<String, Slot>>,
}
pub(super) struct Registration {
    service: Arc<Service>,
    host_id: String,
    channel_id: String,
}
#[cfg(test)]
impl Registration {
    pub(super) fn for_test(service: Arc<Service>, host_id: String, channel_id: String) -> Self {
        Self {
            service,
            host_id,
            channel_id,
        }
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut slots) = self.service.exports.slots.lock()
            && slots
                .get(&self.host_id)
                .is_some_and(|s| s.lease.channel_id == self.channel_id)
        {
            slots.remove(&self.host_id);
        }
    }
}
impl Service {
    /// Synthetic responder behind the real Describe queue/factory. This helper
    /// is absent from production and does not simulate a verified WSS socket.
    #[cfg(test)]
    pub(crate) fn register_export_fixture(
        self: &Arc<Self>,
        state: &AppState,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        descriptor: Arc<Mutex<ExportDescriptor>>,
    ) -> Result<(Box<dyn Send>, Arc<std::sync::atomic::AtomicUsize>)> {
        let (registration, mut requests) = self.register_exports(
            state,
            identity,
            lease,
            descriptor.lock().unwrap().scope.controller_node_id.clone(),
        )?;
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = calls.clone();
        let task = tokio::spawn(async move {
            while let Some(request) = requests.recv().await {
                observed.fetch_add(1, Ordering::Release);
                let _ = request
                    .response
                    .send(Ok(descriptor.lock().unwrap().clone()));
            }
        });
        struct Fixture {
            _registration: Registration,
            task: tokio::task::AbortHandle,
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                self.task.abort();
            }
        }
        Ok((
            Box::new(Fixture {
                _registration: registration,
                task: task.abort_handle(),
            }),
            calls,
        ))
    }
    pub(super) fn register_exports(
        self: &Arc<Self>,
        state: &AppState,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        controller_node_id: String,
    ) -> std::result::Result<(Registration, mpsc::Receiver<DescribeRequest>), WorkerErrorCode> {
        let (sender, receiver) = mpsc::channel(PENDING);
        let store = state
            .store
            .lock()
            .map_err(|_| WorkerErrorCode::InternalError)?;
        store
            .validate_worker_channel(identity, lease, &self.boot_id, now_ms()?)
            .map_err(|_| WorkerErrorCode::StaleLease)?;
        let mut slots = self
            .exports
            .slots
            .lock()
            .map_err(|_| WorkerErrorCode::InternalError)?;
        if slots
            .get(identity.host_id())
            .is_some_and(|current| current.lease.epoch >= lease.epoch)
        {
            return Err(WorkerErrorCode::StaleLease);
        }
        slots.insert(
            identity.host_id().into(),
            Slot {
                identity: identity.clone(),
                lease: lease.clone(),
                controller_node_id,
                sender,
                permits: Arc::new(Semaphore::new(PENDING)),
            },
        );
        drop(slots);
        drop(store);
        Ok((
            Registration {
                service: self.clone(),
                host_id: identity.host_id().into(),
                channel_id: lease.channel_id.clone(),
            },
            receiver,
        ))
    }
    #[cfg(test)]
    pub(crate) fn pending_exports_for_test(&self, host_id: &str) -> usize {
        self.exports
            .slots
            .lock()
            .unwrap()
            .get(host_id)
            .map_or(0, |slot| PENDING - slot.permits.available_permits())
    }
    pub(crate) async fn describe_export(
        &self,
        state: &AppState,
        host_id: &str,
        export_id: &str,
        principal: ExportPrincipal,
        expected_revision: Option<u64>,
    ) -> std::result::Result<ObservedExport, DescribeError> {
        let slot = self
            .exports
            .slots
            .lock()
            .map_err(|_| DescribeError::Unavailable)?
            .get(host_id)
            .cloned()
            .ok_or(DescribeError::Unavailable)?;
        let permit = slot
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| DescribeError::Busy)?;
        let request_id = uuid::Uuid::now_v7().to_string();
        ControllerMessage::DescribeExport {
            request_id: request_id.clone(),
            channel_id: slot.lease.channel_id.clone(),
            lease_epoch: slot.lease.epoch,
            export_id: export_id.into(),
            principal: principal.clone(),
            expected_export_revision: expected_revision,
        }
        .validate()
        .map_err(|_| DescribeError::InvalidResponse)?;
        self.validate_export_slot(state, &slot)?;
        let (sender, response) = oneshot::channel();
        let deadline = tokio::time::Instant::now() + REQUEST_TTL;
        slot.sender
            .try_send(DescribeRequest {
                request_id,
                export_id: export_id.into(),
                principal: principal.clone(),
                expected: expected_revision,
                deadline,
                response: sender,
                _permit: permit,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => DescribeError::Busy,
                _ => DescribeError::Unavailable,
            })?;
        let descriptor = tokio::time::timeout_at(deadline, response)
            .await
            .map_err(|_| DescribeError::Unavailable)?
            .map_err(|_| DescribeError::Unavailable)??;
        if tokio::time::Instant::now() >= deadline {
            return Err(DescribeError::Unavailable);
        }
        descriptor
            .validate()
            .map_err(|_| DescribeError::InvalidResponse)?;
        let scope = &descriptor.scope;
        let host = slot.identity.host();
        if scope.controller_node_id != slot.controller_node_id
            || scope.worker_node_id != host.definition.node_id
            || scope.workspace_id != host.workspace_id
            || scope.host_id != host.id
            || scope.enrollment_epoch != host.enrollment_epoch
            || scope.credential_revision != host.credential_revision
            || descriptor.export.id != export_id
            || descriptor.grant.principal != principal
        {
            return Err(DescribeError::InvalidResponse);
        }
        if expected_revision.is_some_and(|r| r != descriptor.export.revision) {
            return Err(DescribeError::StaleExport);
        }
        self.validate_export_slot(state, &slot)?;
        if tokio::time::Instant::now() >= deadline {
            return Err(DescribeError::Unavailable);
        }
        Ok(ObservedExport {
            identity: slot.identity,
            lease: slot.lease,
            descriptor,
            observed_at_ms: now_ms().map_err(|_| DescribeError::Unavailable)?,
            observed: Instant::now(),
        })
    }
    /// Network-only observation factory for an opaque current v3 transport.
    /// It performs no Store/FS operation. Every positive consumer MUST finish
    /// with independent fresh SQL authority + transport attestation afterwards.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Internal v3 compatibility producer; interactive callers use describe_session_export"
        )
    )]
    pub(crate) async fn describe_execution_export(
        &self,
        transport: &super::execution::ExecutionTransport,
        export_id: &str,
        principal: ExportPrincipal,
        expected_revision: Option<u64>,
    ) -> std::result::Result<ObservedExport, DescribeError> {
        self.describe_live_export(transport, export_id, principal, expected_revision)
            .await
    }
    pub(crate) async fn describe_session_export(
        &self,
        transport: &super::sessions::SessionTransport,
        export_id: &str,
        principal: ExportPrincipal,
        expected_revision: Option<u64>,
    ) -> std::result::Result<ObservedExport, DescribeError> {
        self.describe_live_export(transport, export_id, principal, expected_revision)
            .await
    }
    async fn describe_live_export(
        &self,
        transport: &impl LiveTransport,
        export_id: &str,
        principal: ExportPrincipal,
        expected_revision: Option<u64>,
    ) -> std::result::Result<ObservedExport, DescribeError> {
        let slot = self
            .exports
            .slots
            .lock()
            .map_err(|_| DescribeError::Unavailable)?
            .get(transport.host_id())
            .cloned()
            .ok_or(DescribeError::Unavailable)?;
        let current = || {
            !self.shutdown.load(Ordering::Acquire)
                && transport.matches_metadata_slot(
                    &slot.identity,
                    &slot.lease,
                    &slot.controller_node_id,
                )
                && self.exports.slots.lock().is_ok_and(|slots| {
                    slots.get(transport.host_id()).is_some_and(|active| {
                        active.lease.channel_id == slot.lease.channel_id
                            && active.lease.epoch == slot.lease.epoch
                    })
                })
        };
        if !current() {
            return Err(DescribeError::Unavailable);
        }
        let permit = slot
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| DescribeError::Busy)?;
        let request_id = uuid::Uuid::now_v7().to_string();
        ControllerMessage::DescribeExport {
            request_id: request_id.clone(),
            channel_id: slot.lease.channel_id.clone(),
            lease_epoch: slot.lease.epoch,
            export_id: export_id.into(),
            principal: principal.clone(),
            expected_export_revision: expected_revision,
        }
        .validate()
        .map_err(|_| DescribeError::InvalidResponse)?;
        let (response, receive) = oneshot::channel();
        let deadline = tokio::time::Instant::now() + REQUEST_TTL;
        slot.sender
            .try_send(DescribeRequest {
                request_id,
                export_id: export_id.into(),
                principal: principal.clone(),
                expected: expected_revision,
                deadline,
                response,
                _permit: permit,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => DescribeError::Busy,
                _ => DescribeError::Unavailable,
            })?;
        let descriptor = tokio::time::timeout_at(deadline, receive)
            .await
            .map_err(|_| DescribeError::Unavailable)?
            .map_err(|_| DescribeError::Unavailable)??;
        descriptor
            .validate()
            .map_err(|_| DescribeError::InvalidResponse)?;
        let scope = &descriptor.scope;
        let host = slot.identity.host();
        if scope.controller_node_id != slot.controller_node_id
            || scope.worker_node_id != host.definition.node_id
            || scope.workspace_id != host.workspace_id
            || scope.host_id != host.id
            || scope.enrollment_epoch != host.enrollment_epoch
            || scope.credential_revision != host.credential_revision
            || descriptor.export.id != export_id
            || descriptor.grant.principal != principal
        {
            return Err(DescribeError::InvalidResponse);
        }
        if expected_revision.is_some_and(|revision| revision != descriptor.export.revision) {
            return Err(DescribeError::StaleExport);
        }
        if !current() || tokio::time::Instant::now() >= deadline {
            return Err(DescribeError::Unavailable);
        }
        Ok(ObservedExport {
            identity: slot.identity,
            lease: slot.lease,
            descriptor,
            observed_at_ms: now_ms().map_err(|_| DescribeError::Unavailable)?,
            observed: Instant::now(),
        })
    }
    fn validate_export_slot(
        &self,
        state: &AppState,
        slot: &Slot,
    ) -> std::result::Result<(), DescribeError> {
        if self.shutdown.load(Ordering::Acquire)
            || state
                .config
                .worker_controller
                .as_ref()
                .is_none_or(|config| config.node_id != slot.controller_node_id)
        {
            return Err(DescribeError::Unavailable);
        }
        let current = self
            .exports
            .slots
            .lock()
            .map_err(|_| DescribeError::Unavailable)?
            .get(slot.identity.host_id())
            .is_some_and(|s| {
                s.lease.channel_id == slot.lease.channel_id && s.lease.epoch == slot.lease.epoch
            });
        if !current {
            return Err(DescribeError::Unavailable);
        }
        state
            .store
            .lock()
            .map_err(|_| DescribeError::Unavailable)?
            .validate_worker_channel(
                &slot.identity,
                &slot.lease,
                &self.boot_id,
                now_ms().map_err(|_| DescribeError::Unavailable)?,
            )
            .map_err(|_| DescribeError::Unavailable)?;
        Ok(())
    }
}

// Private, sealed by this module: peer JSON cannot implement transport proof.
trait LiveTransport: Send + Sync {
    fn host_id(&self) -> &str;
    fn matches_metadata_slot(
        &self,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        node: &str,
    ) -> bool;
}
impl LiveTransport for super::execution::ExecutionTransport {
    fn host_id(&self) -> &str {
        self.host_id()
    }
    fn matches_metadata_slot(
        &self,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        node: &str,
    ) -> bool {
        self.matches_metadata_slot(identity, lease, node)
    }
}
impl LiveTransport for super::sessions::SessionTransport {
    fn host_id(&self) -> &str {
        self.host_id()
    }
    fn matches_metadata_slot(
        &self,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        node: &str,
    ) -> bool {
        self.matches_metadata_slot(identity, lease, node)
    }
}
pub(super) async fn messages(
    socket: &mut WebSocket,
    state: &AppState,
    identity: &WorkerIdentity,
    lease: &mut ChannelLease,
    rate: &mut Rate,
    receiver: &mut mpsc::Receiver<DescribeRequest>,
    metadata: bool,
) -> Result<()> {
    let mut pending: BTreeMap<String, DescribeRequest> = BTreeMap::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _=tick.tick()=>{
                pending.retain(|_,request|tokio::time::Instant::now()<request.deadline && !request.response.is_closed());
            }
            request=receiver.recv()=>{
                let Some(request)=request else{return Err(WorkerErrorCode::ChannelExpired);};
                if request.response.is_closed() || tokio::time::Instant::now()>=request.deadline {continue;}
                let message=ControllerMessage::DescribeExport{request_id:request.request_id.clone(),channel_id:lease.channel_id.clone(),lease_epoch:lease.epoch,export_id:request.export_id.clone(),principal:request.principal.clone(),expected_export_revision:request.expected};
                pending.insert(request.request_id.clone(),request);
                send(socket,message).await?;
            }
            message=receive(socket,rate)=>{
                match message? {
                    WorkerMessage::Heartbeat{channel_id,lease_epoch,sequence}=>{
                        if channel_id!=lease.channel_id || lease_epoch!=lease.epoch {return Err(WorkerErrorCode::StaleLease);}
                        let received = std::time::Instant::now();
                        *lease=state.store.lock().map_err(|_|WorkerErrorCode::InternalError)?.renew_worker_channel(identity,lease,sequence,now_ms()?,TTL_MS).map_err(|_|WorkerErrorCode::InvalidSequence)?;
                        state.worker_channels.observed_heartbeat(lease, received);
                        send(socket,ControllerMessage::HeartbeatAck{channel_id:lease.channel_id.clone(),lease_epoch:lease.epoch,sequence,expires_at_ms:u64::try_from(lease.expires_at_ms).map_err(|_|WorkerErrorCode::InternalError)?}).await?;
                    }
                    WorkerMessage::ExportDescribed{request_id,channel_id,lease_epoch,descriptor}=>{
                        if !metadata { return Err(WorkerErrorCode::UnsupportedProtocol); }
                        if channel_id!=lease.channel_id || lease_epoch!=lease.epoch {return Err(WorkerErrorCode::StaleLease);}
                        if let Some(request)=pending.remove(&request_id) && tokio::time::Instant::now()<request.deadline {
                            let _=request.response.send(Ok(descriptor));
                        }
                    }
                    WorkerMessage::ExportError{request_id,channel_id,lease_epoch,code}=>{
                        if !metadata { return Err(WorkerErrorCode::UnsupportedProtocol); }
                        if channel_id!=lease.channel_id || lease_epoch!=lease.epoch {return Err(WorkerErrorCode::StaleLease);}
                        if let Some(request)=pending.remove(&request_id) {
                            let error=match code {ExportErrorCode::Unavailable=>DescribeError::Unavailable,ExportErrorCode::Forbidden=>DescribeError::Forbidden,ExportErrorCode::StaleExport=>DescribeError::StaleExport,ExportErrorCode::Busy=>DescribeError::Busy};
                            let _=request.response.send(Err(error));
                        }
                    }
                    _=>return Err(WorkerErrorCode::InvalidMessage),
                }
            }
        }
    }
}
