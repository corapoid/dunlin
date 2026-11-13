//! Session-owned producer registration and bounded socket work. These objects
//! correlate transport only; the host handler must check full authority, SDK
//! consent and native receipts, and retain capacity through real blocking work.
use super::*;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    future::Future,
    pin::Pin,
    sync::{Mutex, Weak},
};
use tokio::{
    sync::Semaphore,
    task::{AbortHandle, JoinSet},
    time::Instant,
};

const SESSIONS: usize = 8;
const REQUEST_IDS: usize = 4096;
const WAIT: Duration = Duration::from_secs(150);
type Answer = std::result::Result<String, wire::BrokerError>;
pub(crate) trait SessionBroker: Send + Sync {
    fn session(&self) -> &str;
    /// No SQL/FS/network here: called by the socket owner before each fragment.
    fn is_live(&self) -> bool;
    /// The future must revalidate the exact request scope and authority before
    /// returning a result. Cancelled waiters never release real HTTP-work slots.
    fn request(
        self: Arc<Self>,
        request: wire::BrokerRequest,
        deadline: Instant,
    ) -> Pin<Box<dyn Future<Output = Answer> + Send>>;
    /// Fast negative signal only, including after uncertain dispatch/timeout.
    fn interrupt(&self);
}
struct Entry {
    token: String,
    handler: Weak<dyn SessionBroker>,
}
pub(in crate::worker_channel) struct Registry {
    alive: AtomicBool,
    entries: Mutex<BTreeMap<String, Entry>>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            alive: AtomicBool::new(true),
            entries: Mutex::new(BTreeMap::new()),
        }
    }
}
#[derive(Clone)]
struct Binding {
    token: String,
    handler: Arc<dyn SessionBroker>,
}
pub(crate) struct BrokerRegistration {
    registry: Weak<Registry>,
    binding: Binding,
}
#[cfg(test)]
impl BrokerRegistration {
    pub(crate) fn weak_owner(&self) -> Weak<dyn SessionBroker> {
        Arc::downgrade(&self.binding.handler)
    }
}
impl Drop for BrokerRegistration {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade()
            && let Ok(mut entries) = registry.entries.lock()
            && entries
                .get(self.binding.handler.session())
                .is_some_and(|entry| entry.token == self.binding.token)
        {
            entries.remove(self.binding.handler.session());
        }
        self.binding.handler.interrupt();
    }
}
impl Registry {
    pub(in crate::worker_channel) fn bind(
        self: &Arc<Self>,
        handler: Arc<dyn SessionBroker>,
    ) -> Result<BrokerRegistration> {
        let session = handler.session();
        if !self.alive.load(Ordering::Acquire) || !handler.is_live() {
            return Err(WorkerErrorCode::ChannelExpired);
        }
        if session.is_empty()
            || session.len() > 128
            || !session
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(WorkerErrorCode::InvalidMessage);
        }
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| WorkerErrorCode::InternalError)?;
        entries.retain(|_, entry| entry.handler.strong_count() > 0);
        if entries.contains_key(session) || entries.len() >= SESSIONS {
            return Err(WorkerErrorCode::CapacityReached);
        }
        let token = uuid::Uuid::now_v7().to_string();
        entries.insert(
            session.into(),
            Entry {
                token: token.clone(),
                handler: Arc::downgrade(&handler),
            },
        );
        if !self.alive.load(Ordering::Acquire) {
            entries.remove(session);
            return Err(WorkerErrorCode::ChannelExpired);
        }
        Ok(BrokerRegistration {
            registry: Arc::downgrade(self),
            binding: Binding { token, handler },
        })
    }
    fn get(&self, session: &str) -> Option<Binding> {
        if !self.alive.load(Ordering::Acquire) {
            return None;
        }
        let entries = self.entries.lock().ok()?;
        let entry = entries.get(session)?;
        Some(Binding {
            token: entry.token.clone(),
            handler: entry.handler.upgrade()?,
        })
    }
    fn current(&self, binding: &Binding) -> bool {
        self.alive.load(Ordering::Acquire)
            && self.entries.lock().is_ok_and(|entries| {
                entries
                    .get(binding.handler.session())
                    .is_some_and(|entry| entry.token == binding.token)
            })
            && binding.handler.is_live()
    }
    pub(in crate::worker_channel) fn stop(&self) {
        self.alive.store(false, Ordering::Release);
        let entries = self
            .entries
            .lock()
            .map(|mut entries| std::mem::take(&mut *entries))
            .unwrap_or_default();
        for entry in entries.into_values() {
            if let Some(handler) = entry.handler.upgrade() {
                handler.interrupt();
            }
        }
    }
}

struct Pending {
    request: wire::BrokerRequest,
    binding: Binding,
    deadline: Instant,
    task: AbortHandle,
    replies: VecDeque<wire::BrokerControllerMessage>,
    _slot: OwnedSemaphorePermit,
}
struct Completed {
    id: String,
    result: Answer,
}
pub(in crate::worker_channel) struct Exchange {
    transport: sessions::SessionTransport,
    registry: Arc<Registry>,
    slots: Arc<Semaphore>,
    pending: BTreeMap<String, Pending>,
    seen: BTreeSet<String>,
    jobs: JoinSet<Completed>,
}
fn rejected(
    request: &wire::BrokerRequest,
    code: wire::BrokerError,
) -> Result<wire::BrokerControllerMessage> {
    Ok(wire::BrokerControllerMessage::McpRejected {
        request: request
            .reference()
            .map_err(|_| WorkerErrorCode::InvalidMessage)?,
        code,
    })
}
impl Exchange {
    pub(in crate::worker_channel) fn new(
        transport: sessions::SessionTransport,
        registry: Arc<Registry>,
    ) -> Self {
        let capacity = transport
            .broker_offer()
            .map_or(0, |limits| usize::from(limits.max_pending));
        Self {
            transport,
            registry,
            slots: Arc::new(Semaphore::new(capacity)),
            pending: BTreeMap::new(),
            seen: BTreeSet::new(),
            jobs: JoinSet::new(),
        }
    }
    pub(in crate::worker_channel) fn active(&self) -> bool {
        !self.pending.is_empty() || !self.jobs.is_empty()
    }
    pub(in crate::worker_channel) fn submit(
        &mut self,
        request: wire::BrokerRequest,
    ) -> Result<Option<wire::BrokerControllerMessage>> {
        request
            .validate_for_channel(self.transport.fence())
            .map_err(|_| WorkerErrorCode::InvalidMessage)?;
        if !self.transport.is_live() {
            return Err(WorkerErrorCode::ChannelExpired);
        }
        let Some(binding) = self
            .registry
            .get(&request.scope.session().controller_session_id)
            .filter(|binding| self.registry.current(binding))
        else {
            return rejected(&request, wire::BrokerError::Denied).map(Some);
        };
        // Never replay a dispatched ID, even after its result has left pending.
        if self.seen.contains(&request.request_id) {
            return Err(WorkerErrorCode::InvalidSequence);
        }
        if self.seen.len() >= REQUEST_IDS {
            return Err(WorkerErrorCode::CapacityReached);
        }
        let Ok(slot) = self.slots.clone().try_acquire_owned() else {
            return rejected(&request, wire::BrokerError::CapacityReached).map(Some);
        };
        self.seen.insert(request.request_id.clone());
        let deadline = Instant::now() + WAIT;
        let handler = binding.handler.clone();
        let work = request.clone();
        let task = self.jobs.spawn(async move {
            let result = match tokio::time::timeout_at(
                deadline,
                handler.clone().request(work.clone(), deadline),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    handler.interrupt();
                    Err(wire::BrokerError::OutcomeUnknown)
                }
            };
            Completed {
                id: work.request_id,
                result,
            }
        });
        self.pending.insert(
            request.request_id.clone(),
            Pending {
                request,
                binding,
                deadline,
                task,
                replies: VecDeque::new(),
                _slot: slot,
            },
        );
        Ok(None)
    }
    pub(in crate::worker_channel) fn expire(&mut self) -> Result<()> {
        self.expire_at(Instant::now())
    }
    fn expire_at(&mut self, now: Instant) -> Result<()> {
        if !self.transport.is_live() {
            self.registry.stop();
            return Err(WorkerErrorCode::ChannelExpired);
        }
        for pending in self.pending.values_mut() {
            if now >= pending.deadline || !self.registry.current(&pending.binding) {
                pending.binding.handler.interrupt();
                pending.task.abort();
                pending.replies = VecDeque::from([rejected(
                    &pending.request,
                    wire::BrokerError::OutcomeUnknown,
                )?]);
            }
        }
        Ok(())
    }
    fn complete(&mut self, completion: Completed) -> Result<()> {
        let Some(pending) = self.pending.get_mut(&completion.id) else {
            return Ok(());
        };
        if !pending.replies.is_empty() {
            return Ok(());
        }
        let maximum = self
            .transport
            .broker_offer()
            .ok_or(WorkerErrorCode::InvalidMessage)?
            .max_reply_bytes as usize;
        let result =
            if Instant::now() >= pending.deadline || !self.registry.current(&pending.binding) {
                Err(wire::BrokerError::OutcomeUnknown)
            } else {
                completion.result
            };
        pending.replies = match result {
            Ok(payload) if payload.len() <= maximum => {
                match wire::chunk_reply(&pending.request, &payload) {
                    Ok(chunks) => chunks
                        .into_iter()
                        .map(|chunk| wire::BrokerControllerMessage::McpReplyChunk { chunk })
                        .collect(),
                    Err(_) => {
                        pending.binding.handler.interrupt();
                        VecDeque::from([rejected(
                            &pending.request,
                            wire::BrokerError::OutcomeUnknown,
                        )?])
                    }
                }
            }
            Ok(_) => {
                pending.binding.handler.interrupt();
                VecDeque::from([rejected(
                    &pending.request,
                    wire::BrokerError::OutcomeUnknown,
                )?])
            }
            Err(code) => {
                if code == wire::BrokerError::OutcomeUnknown {
                    pending.binding.handler.interrupt();
                }
                VecDeque::from([rejected(&pending.request, code)?])
            }
        };
        Ok(())
    }
    fn frame(&mut self) -> Result<Option<wire::BrokerControllerMessage>> {
        self.expire()?;
        let Some(id) = self
            .pending
            .iter()
            .find(|(_, pending)| !pending.replies.is_empty())
            .map(|(id, _)| id.clone())
        else {
            return Ok(None);
        };
        let pending = self
            .pending
            .get_mut(&id)
            .ok_or(WorkerErrorCode::InternalError)?;
        let reply = pending
            .replies
            .pop_front()
            .ok_or(WorkerErrorCode::InternalError)?;
        reply
            .validate_for_request(&pending.request)
            .map_err(|_| WorkerErrorCode::InternalError)?;
        if pending.replies.is_empty() {
            self.pending.remove(&id);
        }
        Ok(Some(reply))
    }
    /// Cancellation-safe socket select branch: at most one ready fragment;
    /// joins HTTP/approval work only when ready, never inline on the owner.
    pub(in crate::worker_channel) async fn next(
        &mut self,
    ) -> Result<wire::BrokerControllerMessage> {
        loop {
            if let Some(frame) = self.frame()? {
                return Ok(frame);
            }
            match self.jobs.join_next().await {
                Some(Ok(completion)) => self.complete(completion)?,
                Some(Err(error)) if error.is_cancelled() => {}
                Some(Err(_)) => return Err(WorkerErrorCode::InternalError),
                None => return std::future::pending().await,
            }
        }
    }
}
impl Drop for Exchange {
    fn drop(&mut self) {
        for pending in self.pending.values() {
            pending.binding.handler.interrupt();
        }
        self.jobs.abort_all();
    }
}

#[cfg(test)]
mod tests;
