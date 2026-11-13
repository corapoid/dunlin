//! V4/v5 socket jobs own no process authority. All SQL/FS/ACP work stays in the
//! runtime; channel expiry/drop independently fences and stops that runtime.
use super::*;
use crate::worker_execution::runtime::{Connected, Runtime};
use dunlin_relay_protocol::worker::{
    ExecutionErrorCode as Code,
    session::{SessionControllerMessage as Request, SessionWorkerMessage as Reply},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicBool, Ordering},
};

mod control;
use control::Control;

struct Validity {
    open: AtomicBool,
    deadline: Mutex<tokio::time::Instant>,
}
impl Validity {
    fn live(&self) -> bool {
        self.open.load(Ordering::Acquire)
            && self
                .deadline
                .lock()
                .is_ok_and(|deadline| tokio::time::Instant::now() < *deadline)
    }
}
enum Completed {
    Reply {
        request: Box<Control>,
        result: Box<Result<Option<Reply>, Code>>,
    },
    Outbox(Result<Vec<Reply>, Code>),
}
pub(super) struct Requests {
    runtime: Arc<Runtime>,
    connection: Connected,
    validity: Arc<Validity>,
    fence: dunlin_relay_protocol::worker::ChannelFence,
    offer: Option<dunlin_relay_protocol::worker::session::broker::BrokerLimits>,
    jobs: tokio::task::JoinSet<Completed>,
    pending: BTreeSet<String>,
    polling: bool,
    sent: BTreeMap<(String, u64), (String, tokio::time::Instant)>,
    ready: BTreeSet<String>,
    turns: BTreeMap<
        (String, String),
        (
            dunlin_relay_protocol::worker::session::TurnReceipt,
            tokio::time::Instant,
        ),
    >,
}
impl Requests {
    pub fn new(
        runtime: Arc<Runtime>,
        fence: dunlin_relay_protocol::worker::ChannelFence,
        deadline: tokio::time::Instant,
    ) -> Result<Self, &'static str> {
        Self::connected(runtime, fence, deadline, None).map(|(requests, _)| requests)
    }
    pub(super) fn new_broker(
        runtime: Arc<Runtime>,
        fence: dunlin_relay_protocol::worker::ChannelFence,
        deadline: tokio::time::Instant,
        limits: dunlin_relay_protocol::worker::session::broker::BrokerLimits,
    ) -> Result<(Self, broker::SocketOwner), &'static str> {
        let (requests, owner) = Self::connected(runtime, fence, deadline, Some(limits))?;
        Ok((requests, owner.ok_or("worker_broker_owner_missing")?))
    }
    fn connected(
        runtime: Arc<Runtime>,
        fence: dunlin_relay_protocol::worker::ChannelFence,
        deadline: tokio::time::Instant,
        limits: Option<dunlin_relay_protocol::worker::session::broker::BrokerLimits>,
    ) -> Result<(Self, Option<broker::SocketOwner>), &'static str> {
        if !runtime.supports_sessions() {
            return Err("worker_session_runtime_required");
        }
        let validity = Arc::new(Validity {
            open: AtomicBool::new(true),
            deadline: Mutex::new(deadline),
        });
        let offer = limits.clone();
        let transport_guard = validity.clone();
        let pair = limits
            .map(|limits| {
                broker::SocketOwner::new(
                    fence.clone(),
                    limits,
                    Arc::new(move || {
                        if transport_guard.live() {
                            Ok(())
                        } else {
                            Err(broker::Error::Closed)
                        }
                    }),
                )
            })
            .transpose()
            .map_err(|_| "worker_broker_channel_unavailable")?;
        let guard = validity.clone();
        let guard = Arc::new(move || {
            if guard.live() {
                Ok(())
            } else {
                Err(crate::worker_exports::Error::Denied)
            }
        });
        let (connection, owner) = match pair {
            Some((client, owner)) => (
                runtime.broker_connection(fence.clone(), guard, client),
                Some(owner),
            ),
            None => (runtime.connection(fence.clone(), guard), None),
        };
        let connection = connection.map_err(|_| "worker_session_channel_unavailable")?;
        Ok((
            Self {
                runtime,
                connection,
                validity,
                fence,
                offer,
                jobs: tokio::task::JoinSet::new(),
                pending: BTreeSet::new(),
                polling: false,
                sent: BTreeMap::new(),
                ready: BTreeSet::new(),
                turns: BTreeMap::new(),
            },
            owner,
        ))
    }
    pub fn deadline(&self, deadline: tokio::time::Instant) -> Result<(), &'static str> {
        let mut old = self
            .validity
            .deadline
            .lock()
            .map_err(|_| "worker_session_guard_unavailable")?;
        if !self.validity.open.load(Ordering::Acquire) || tokio::time::Instant::now() >= *old {
            return Err("worker_session_channel_expired");
        }
        *old = deadline;
        Ok(())
    }
    pub fn pending(&self) -> bool {
        !self.jobs.is_empty()
    }
    pub fn enqueue(&mut self, request: Request) -> Result<Option<Reply>, &'static str> {
        self.enqueue_control(Control::Legacy(Box::new(request)))
    }
    pub(super) fn enqueue_broker(
        &mut self,
        command: dunlin_relay_protocol::worker::session::broker::OpenBrokerSessionCommand,
    ) -> Result<Option<Reply>, &'static str> {
        self.enqueue_control(Control::Broker(Box::new(command)))
    }
    fn enqueue_control(&mut self, request: Control) -> Result<Option<Reply>, &'static str> {
        request.validate(&self.fence, self.offer.as_ref())?;
        let key = request.key()?;
        if self.pending.contains(&key) || self.pending.len() >= 8 {
            return Ok(Some(request.rejected(Code::CapacityReached)));
        }
        self.pending.insert(key);
        let runtime = self.runtime.clone();
        let connection = self.connection.clone();
        self.jobs.spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                match &request {
                    Control::Legacy(request) => {
                        runtime.handle_session(&connection, *request.clone()).await
                    }
                    Control::Broker(command) => {
                        runtime
                            .handle_broker_open(&connection, *command.clone())
                            .await
                    }
                }
            })
            .await
            .unwrap_or(Err(Code::Unavailable));
            Completed::Reply {
                request: Box::new(request),
                result: Box::new(result),
            }
        });
        Ok(None)
    }
    pub fn poll_outbox(&mut self) {
        if self.polling {
            return;
        }
        self.polling = true;
        let runtime = self.runtime.clone();
        let connection = self.connection.clone();
        self.jobs.spawn(async move {
            Completed::Outbox(
                tokio::time::timeout(Duration::from_secs(5), runtime.session_outbox(&connection))
                    .await
                    .unwrap_or(Err(Code::Unavailable)),
            )
        });
    }
    pub async fn completed(&mut self) -> Result<Vec<Reply>, &'static str> {
        match self
            .jobs
            .join_next()
            .await
            .ok_or("worker_session_job_missing")?
            .map_err(|_| "worker_session_job_failed")?
        {
            Completed::Reply { request, result } => {
                let request = *request;
                let result = *result;
                self.pending.remove(&request.key()?);
                match result {
                    Ok(Some(reply)) => {
                        if !request.accepts(&reply) {
                            return Err("worker_session_reply_mismatch");
                        }
                        if let Reply::StreamAcknowledged {
                            session,
                            through_sequence,
                            ..
                        } = &reply
                        {
                            self.sent.retain(|(id, sequence), _| {
                                id != &session.worker_session_id || sequence > through_sequence
                            });
                        }
                        Ok(vec![reply])
                    }
                    Ok(None) => Ok(vec![]),
                    Err(code) => Ok(vec![request.rejected(code)]),
                }
            }
            Completed::Outbox(result) => {
                self.polling = false;
                let mut replies = Vec::new();
                for reply in result.map_err(|_| "worker_session_outbox_unavailable")? {
                    reply
                        .validate_for_channel(&self.fence)
                        .map_err(|_| "worker_session_outbox_invalid")?;
                    let deliver = match &reply {
                        Reply::SessionReady { receipt } => {
                            self.ready.insert(receipt.fence.worker_session_id.clone())
                        }
                        Reply::TurnAccepted { receipt } if self.offer.is_some() => {
                            let key = (
                                receipt.fence.session.worker_session_id.clone(),
                                receipt.fence.turn_id.clone(),
                            );
                            let now = tokio::time::Instant::now();
                            let deliver = match self.turns.get(&key) {
                                Some((previous, _)) if previous != receipt => {
                                    return Err("worker_session_turn_receipt_changed");
                                }
                                Some((_, last)) => {
                                    now.duration_since(*last) >= Duration::from_millis(500)
                                }
                                None => true,
                            };
                            if deliver {
                                self.turns.insert(key, (receipt.clone(), now));
                            }
                            deliver
                        }
                        Reply::Event { event } => {
                            let key = (event.session.worker_session_id.clone(), event.sequence);
                            let now = tokio::time::Instant::now();
                            let deliver = match self.sent.get(&key) {
                                Some((digest, _)) if digest != &event.digest => {
                                    return Err("worker_session_stream_digest_changed");
                                }
                                Some((_, last)) => {
                                    now.duration_since(*last) >= Duration::from_secs(2)
                                }
                                None => true,
                            };
                            if deliver {
                                self.sent.insert(key, (event.digest.clone(), now));
                            }
                            deliver
                        }
                        _ => return Err("worker_session_outbox_invalid"),
                    };
                    if self.ready.len() > 1024 || self.sent.len() > 4096 || self.turns.len() > 4096
                    {
                        return Err("worker_session_outbox_capacity");
                    }
                    if deliver {
                        replies.push(reply);
                    }
                }
                Ok(replies)
            }
        }
    }
}
impl Drop for Requests {
    fn drop(&mut self) {
        self.validity.open.store(false, Ordering::Release);
        self.runtime.disconnect(&self.connection);
        self.jobs.abort_all();
    }
}
