//! Bounded v5 socket work, independent of session jobs and heartbeat renewal.
//! This is correlation, not execution authority. The runtime must validate its
//! native export/session/turn leases before submission and before using results.
//! The verified v5 socket owns this dispatcher and the runtime connection borrows
//! its live client. Native proxy/session/turn authorization is still separate.
use dunlin_relay_protocol::worker::{
    ChannelFence,
    session::broker::{
        BrokerControllerMessage, BrokerError, BrokerLimits, BrokerOperation, BrokerRequest,
        BrokerScope, BrokerWorkerMessage, ReplyAssembly,
    },
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    time::Instant,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Closed,
    Capacity,
    Expired,
    Invalid,
    Rejected(BrokerError),
    OutcomeUnknown,
}
type Result<T> = std::result::Result<T, Error>;
// Cheap transport freshness only; blocking SQL/FS stays outside the socket loop.
type ChannelGuard = Arc<dyn Fn() -> Result<()> + Send + Sync>;
type Answer = oneshot::Receiver<Result<String>>;
const MAX_WAIT: Duration = Duration::from_secs(150);

struct Pending {
    request: BrokerRequest,
    assembly: ReplyAssembly,
    reply_limit: u32,
    deadline: Instant,
    reply: oneshot::Sender<Result<String>>,
    // The actual queued/dispatched work owns capacity, never the waiting caller.
    _slot: OwnedSemaphorePermit,
}
#[derive(Clone)]
pub(crate) struct Client {
    fence: ChannelFence,
    guard: ChannelGuard,
    limits: BrokerLimits,
    slots: Arc<Semaphore>,
    queue: mpsc::Sender<Pending>,
}
#[derive(Debug)]
pub(super) struct Outbound {
    pub(super) message: BrokerWorkerMessage,
    pub(super) deadline: Instant,
}
pub(super) struct SocketOwner {
    client: Client,
    queue: mpsc::Receiver<Pending>,
    pending: BTreeMap<String, Pending>,
    limits: BrokerLimits,
    closed: bool,
}
impl SocketOwner {
    pub(super) fn limits(&self) -> &BrokerLimits {
        &self.limits
    }

    /// Only the authenticated socket owner may create this pair. Neither peer
    /// JSON nor a Client can mint execution authority or reopen a closed pair.
    pub(super) fn new(
        fence: ChannelFence,
        limits: BrokerLimits,
        guard: ChannelGuard,
    ) -> Result<(Client, Self)> {
        fence.validate().map_err(|_| Error::Invalid)?;
        limits.validate().map_err(|_| Error::Invalid)?;
        guard()?;
        let (sender, queue) = mpsc::channel(usize::from(limits.max_pending));
        let client = Client {
            fence,
            guard,
            limits: limits.clone(),
            slots: Arc::new(Semaphore::new(usize::from(limits.max_pending))),
            queue: sender,
        };
        Ok((
            client.clone(),
            Self {
                client,
                queue,
                pending: BTreeMap::new(),
                limits,
                closed: false,
            },
        ))
    }
    /// Poll in the socket select loop; this never waits for an MCP response or
    /// a human decision. Once returned, even an uncertain socket send must not
    /// be retried. Dropping the owner resolves dispatched requests as unknown.
    pub(super) async fn next(&mut self) -> Result<Outbound> {
        if self.closed {
            return Err(Error::Closed);
        }
        while let Some(pending) = self.queue.recv().await {
            if let Err(error) = (self.client.guard)() {
                let _ = pending.reply.send(Err(error));
                self.close();
                return Err(error);
            }
            if pending.reply.is_closed() {
                continue;
            }
            if Instant::now() >= pending.deadline {
                let _ = pending.reply.send(Err(Error::Expired));
                continue;
            }
            let message = BrokerWorkerMessage::McpRequest {
                request: pending.request.clone(),
            };
            let deadline = pending.deadline;
            self.pending
                .insert(pending.request.request_id.clone(), pending);
            return Ok(Outbound { message, deadline });
        }
        Err(Error::Closed)
    }
    /// Parse only broker replies. Other bounded controller/session frames stay
    /// with their own decoder; malformed replies cannot enter those protocols.
    pub(super) fn accept_frame(&mut self, text: &str) -> Result<bool> {
        if text.len() > dunlin_relay_protocol::worker::MAX_WORKER_FRAME_BYTES {
            self.close();
            return Err(Error::Invalid);
        }
        match serde_json::from_str::<BrokerControllerMessage>(text) {
            Ok(message) => {
                self.accept(message)?;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }
    pub(super) fn accept(&mut self, message: BrokerControllerMessage) -> Result<()> {
        let result = self.accept_inner(message);
        if result.is_err() {
            self.close();
        }
        result
    }
    fn accept_inner(&mut self, message: BrokerControllerMessage) -> Result<()> {
        if self.closed {
            return Err(Error::Closed);
        }
        (self.client.guard)()?;
        let id = match &message {
            BrokerControllerMessage::McpReplyChunk { chunk } => &chunk.request.request_id,
            BrokerControllerMessage::McpRejected { request, .. } => &request.request_id,
        };
        let pending = self.pending.get_mut(id).ok_or(Error::Invalid)?;
        message
            .validate_for_request(&pending.request)
            .map_err(|_| Error::Invalid)?;
        if Instant::now() >= pending.deadline {
            return Err(Error::OutcomeUnknown);
        }
        let answer = match &message {
            BrokerControllerMessage::McpReplyChunk { chunk } => {
                // Check the negotiated limit before ReplyAssembly reserves memory.
                if chunk.total_bytes > pending.reply_limit {
                    return Err(Error::Invalid);
                }
                pending
                    .assembly
                    .push(chunk)
                    .map_err(|_| Error::Invalid)?
                    .map(Ok)
            }
            BrokerControllerMessage::McpRejected { code, .. } => Some(Err(Error::Rejected(*code))),
        };
        if let Some(answer) = answer {
            let pending = self.pending.remove(id).ok_or(Error::Invalid)?;
            let _ = pending.reply.send(answer);
        }
        Ok(())
    }
    /// Call on an independent socket timer, including while no replies arrive.
    /// Any error must fence the owning socket/runtime, not reopen this pair.
    pub(super) fn expire(&mut self) -> Result<()> {
        if let Err(error) = (self.client.guard)() {
            self.close();
            return Err(error);
        }
        self.expire_at(Instant::now())
    }
    fn expire_at(&mut self, now: Instant) -> Result<()> {
        if self.closed {
            return Err(Error::Closed);
        }
        if self.pending.values().any(|pending| now >= pending.deadline) {
            // A sent request may already have changed external state. Stop the
            // pair rather than accepting new work after an uncertain outcome.
            // Dropped senders map to OutcomeUnknown in Client::request.
            self.close();
            return Err(Error::OutcomeUnknown);
        }
        Ok(())
    }
    fn close(&mut self) {
        self.closed = true;
        self.queue.close();
        self.client.slots.close();
        self.pending.clear();
        while let Ok(pending) = self.queue.try_recv() {
            let _ = pending.reply.send(Err(Error::Closed));
        }
    }
}
impl Drop for SocketOwner {
    fn drop(&mut self) {
        self.close();
    }
}
impl Client {
    pub(crate) fn attest(&self, fence: &ChannelFence) -> Result<()> {
        (self.guard)()?;
        if fence != &self.fence {
            return Err(Error::Invalid);
        }
        if self.queue.is_closed() || self.slots.is_closed() {
            return Err(Error::Closed);
        }
        Ok(())
    }
    pub(crate) fn limits(&self) -> &BrokerLimits {
        &self.limits
    }
    #[cfg(test)]
    fn submit(
        &self,
        scope: BrokerScope,
        operation: BrokerOperation,
        deadline: Instant,
    ) -> Result<Answer> {
        self.submit_bounded(scope, operation, deadline, self.limits.max_reply_bytes)
    }
    fn submit_bounded(
        &self,
        scope: BrokerScope,
        operation: BrokerOperation,
        deadline: Instant,
        reply_limit: u32,
    ) -> Result<Answer> {
        if reply_limit == 0 || reply_limit > self.limits.max_reply_bytes {
            return Err(Error::Invalid);
        }

        self.attest(&self.fence)?;
        let now = Instant::now();
        if now >= deadline {
            return Err(Error::Expired);
        }
        if deadline.duration_since(now) > MAX_WAIT {
            return Err(Error::Invalid);
        }
        let request = BrokerRequest {
            scope,
            request_id: uuid::Uuid::now_v7().to_string(),
            operation,
        };
        request
            .validate_for_channel(&self.fence)
            .map_err(|_| Error::Invalid)?;
        let slot = self.slots.clone().try_acquire_owned().map_err(|_| {
            if self.slots.is_closed() {
                Error::Closed
            } else {
                Error::Capacity
            }
        })?;
        let (reply, answer) = oneshot::channel();
        let pending = Pending {
            assembly: ReplyAssembly::new(&request).map_err(|_| Error::Invalid)?,
            reply_limit,
            request,
            deadline,
            reply,
            _slot: slot,
        };
        self.queue.try_send(pending).map_err(|error| match error {
            mpsc::error::TrySendError::Closed(_) => Error::Closed,
            mpsc::error::TrySendError::Full(_) => Error::Capacity,
        })?;
        Ok(answer)
    }
    pub(crate) async fn request(
        &self,
        scope: BrokerScope,
        operation: BrokerOperation,
        deadline: Instant,
    ) -> Result<String> {
        self.request_bounded(scope, operation, deadline, self.limits.max_reply_bytes)
            .await
    }
    pub(crate) async fn request_bounded(
        &self,
        scope: BrokerScope,
        operation: BrokerOperation,
        deadline: Instant,
        reply_limit: u32,
    ) -> Result<String> {
        let answer = self.submit_bounded(scope, operation, deadline, reply_limit)?;
        let result = tokio::time::timeout_at(deadline, answer)
            .await
            .map_err(|_| Error::OutcomeUnknown)?
            .map_err(|_| Error::OutcomeUnknown)?;
        // This is only channel freshness. Native authority must also be checked
        // by the runtime before exposing the answer to its harness.
        self.attest(&self.fence)?;
        if Instant::now() >= deadline {
            return Err(Error::OutcomeUnknown);
        }
        result
    }
}

#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
mod tests;
