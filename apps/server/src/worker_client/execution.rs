//! Operator-only runtime startup and bounded background execution jobs. The WS
//! owner never runs SQLite, checkout validation, ACP or actor work inline.
use super::*;
use crate::worker_execution::runtime::{Connected, LocalHarness, Runtime, RuntimeConfig};
use dunlin_relay_protocol::worker::{
    self as wire, ExecutionControllerMessage as Request, ExecutionWorkerMessage as Reply,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperatorExecutionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub persistent_sessions: bool,
    #[serde(default)]
    pub mcp_broker: bool,
    #[serde(default)]
    pub harnesses: BTreeMap<String, OperatorHarness>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OperatorHarness {
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
}
pub(super) fn validate(config: &WorkerConfig) -> Result<(), &'static str> {
    if config
        .execution
        .as_ref()
        .is_some_and(|e| e.mcp_broker && (!e.enabled || !e.persistent_sessions))
    {
        return Err("worker_broker_session_opt_in_required");
    }
    if config
        .execution
        .as_ref()
        .is_some_and(|execution| execution.persistent_sessions && !execution.enabled)
    {
        return Err("worker_session_execution_opt_in_required");
    }
    let Some(execution) = config.execution.as_ref().filter(|e| e.enabled) else {
        return Ok(());
    };
    if config.exports_state_dir.is_none()
        || execution.harnesses.is_empty()
        || execution.harnesses.len() > 3
        || config.capabilities.sandbox != dunlin_domain::hosts::SandboxStrength::LinuxBubblewrap
        || config.capabilities.max_parallel != 1
    {
        return Err("worker_execution_configuration_unsupported");
    }
    for (id, harness) in &execution.harnesses {
        if !dunlin_domain::agent::AGENT_HARNESS_IDS.contains(&id.as_str())
            || !config.capabilities.harnesses.contains(id)
            || !harness.program.is_absolute()
            || harness.args.len() > 32
            || harness.args.iter().map(String::len).sum::<usize>() > 16384
            || harness.args.iter().any(|v| v.contains('\0'))
        {
            return Err("worker_execution_configuration_unsupported");
        }
    }
    Ok(())
}
pub(super) async fn start(
    config: &WorkerConfig,
    boot: &str,
) -> Result<Option<Arc<Runtime>>, &'static str> {
    validate(config)?;
    let Some(execution) = config.execution.as_ref().filter(|e| e.enabled) else {
        return Ok(None);
    };
    let runtime = RuntimeConfig {
        state_dir: config
            .exports_state_dir
            .clone()
            .ok_or("worker_execution_state_required")?,
        scope: crate::worker_exports::ExportScope {
            controller_node_id: config.controller.node_id.clone(),
            worker_node_id: config.network.node_id.clone(),
            workspace_id: config.workspace_id.clone(),
            host_id: config.host_id.clone(),
            enrollment_epoch: config.enrollment_epoch,
            credential_revision: config.credential_revision,
        },
        worker_boot_id: boot.into(),
        server_config: crate::config::Config {
            sandbox: true,
            ..Default::default()
        },
        harnesses: execution
            .harnesses
            .iter()
            .map(|(id, h)| {
                (
                    id.clone(),
                    LocalHarness {
                        program: h.program.clone(),
                        args: h.args.clone(),
                    },
                )
            })
            .collect(),
    };
    let sessions = execution.persistent_sessions;
    tokio::task::spawn_blocking(move || {
        let runtime = Runtime::new(runtime)?;
        if sessions && !runtime.supports_sessions() {
            return Err(wire::ExecutionErrorCode::UnsupportedProfile);
        }
        Ok(runtime)
    })
    .await
    .map_err(|_| "worker_execution_startup_failed")?
    .map(Some)
    .map_err(|_| "worker_execution_startup_failed")
}
struct Validity {
    open: AtomicBool,
    deadline: Mutex<tokio::time::Instant>,
}
pub(super) struct Requests {
    runtime: Arc<Runtime>,
    connection: Connected,
    validity: Arc<Validity>,
    fence: wire::ChannelFence,
    tasks: tokio::task::JoinSet<Completed>,
    ids: BTreeSet<String>,
    outbox_pending: bool,
    sent: BTreeMap<String, tokio::time::Instant>,
}
pub(super) enum Completed {
    Reply {
        request: Box<Request>,
        result: Box<Result<Option<Reply>, wire::ExecutionErrorCode>>,
    },
    Outbox(Result<Vec<wire::ExecutionRecord>, wire::ExecutionErrorCode>),
}
fn request_key(request: &Request) -> String {
    match request {
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
impl Requests {
    pub fn new(
        runtime: Arc<Runtime>,
        fence: wire::ChannelFence,
        deadline: tokio::time::Instant,
    ) -> Result<Self, &'static str> {
        let validity = Arc::new(Validity {
            open: AtomicBool::new(true),
            deadline: Mutex::new(deadline),
        });
        let guard = validity.clone();
        let connection = runtime
            .connection(
                fence.clone(),
                Arc::new(move || {
                    if !guard.open.load(Ordering::Acquire)
                        || !guard
                            .deadline
                            .lock()
                            .is_ok_and(|d| tokio::time::Instant::now() < *d)
                    {
                        return Err(crate::worker_exports::Error::Denied);
                    }
                    Ok(())
                }),
            )
            .map_err(|_| "worker_execution_channel_unavailable")?;
        Ok(Self {
            runtime,
            connection,
            validity,
            fence,
            tasks: tokio::task::JoinSet::new(),
            ids: BTreeSet::new(),
            outbox_pending: false,
            sent: BTreeMap::new(),
        })
    }
    pub fn deadline(&self, deadline: tokio::time::Instant) -> Result<(), &'static str> {
        *self
            .validity
            .deadline
            .lock()
            .map_err(|_| "worker_execution_guard_unavailable")? = deadline;
        Ok(())
    }
    pub fn pending(&self) -> bool {
        !self.tasks.is_empty()
    }
    pub fn enqueue(&mut self, request: Request) -> Result<Option<Reply>, &'static str> {
        request
            .validate()
            .map_err(|_| "invalid_execution_message")?;
        let channel = match &request {
            Request::Execute { command } => &command.channel,
            Request::Renew { fence, .. } | Request::Cancel { fence } => &fence.channel,
            Request::Reconcile { channel, .. } => channel,
            Request::OutcomeAck { channel, .. } => channel,
        };
        if channel != &self.fence {
            return Err("worker_execution_fence_mismatch");
        }
        let key = request_key(&request);
        if self.ids.contains(&key) {
            if matches!(request, Request::OutcomeAck { .. }) {
                return Ok(None);
            }
            return Err("worker_duplicate_execution_request");
        }
        if self.ids.len() >= 8 {
            return Ok(Some(reject(
                &request,
                wire::ExecutionErrorCode::CapacityReached,
            )));
        }
        self.ids.insert(key);
        let runtime = self.runtime.clone();
        let connection = self.connection.clone();
        self.tasks.spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(5),
                runtime.handle(&connection, request.clone()),
            )
            .await
            .unwrap_or(Err(wire::ExecutionErrorCode::Unavailable));
            Completed::Reply {
                request: Box::new(request),
                result: Box::new(result),
            }
        });
        Ok(None)
    }
    pub fn poll_outbox(&mut self) {
        if self.outbox_pending {
            return;
        }
        self.outbox_pending = true;
        let runtime = self.runtime.clone();
        let connection = self.connection.clone();
        self.tasks.spawn(async move {
            Completed::Outbox(
                tokio::time::timeout(Duration::from_secs(5), runtime.outbox(&connection))
                    .await
                    .unwrap_or(Err(wire::ExecutionErrorCode::Unavailable)),
            )
        });
    }
    pub async fn completed(&mut self) -> Result<Vec<Reply>, &'static str> {
        match self
            .tasks
            .join_next()
            .await
            .ok_or("worker_execution_job_missing")?
            .map_err(|_| "worker_execution_job_failed")?
        {
            Completed::Reply { request, result } => {
                let request = *request;
                let result = *result;
                self.ids.remove(&request_key(&request));
                match result {
                    Ok(Some(reply)) => Ok(vec![reply]),
                    Ok(None) => Ok(vec![]),
                    Err(code) => Ok(vec![reject(&request, code)]),
                }
            }
            Completed::Outbox(result) => {
                self.outbox_pending = false;
                let records = result.map_err(|_| "worker_execution_outbox_unavailable")?;
                let mut replies = Vec::new();
                for record in records {
                    record
                        .validate()
                        .map_err(|_| "worker_execution_outbox_invalid")?;
                    // Historical fences are readable only through an explicit
                    // Reconcile. Never turn a restarted old attempt into a live
                    // unsolicited event or positive work on this connection.
                    if record.receipt.fence.channel != self.fence {
                        continue;
                    }
                    let outcome = record
                        .outcome
                        .as_ref()
                        .ok_or("worker_execution_outbox_nonterminal")?;
                    let key = format!(
                        "{}:{}:{}",
                        record.receipt.identity.attempt_id, outcome.outcome_id, outcome.digest
                    );
                    let now = tokio::time::Instant::now();
                    if !self
                        .sent
                        .get(&key)
                        .is_some_and(|last| now.duration_since(*last) < Duration::from_secs(10))
                    {
                        self.sent.insert(key, now);
                        replies.push(Reply::Accepted { record });
                    }
                    if self.sent.len() > 1024 {
                        return Err("worker_execution_outbox_capacity");
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
        self.tasks.abort_all();
    }
}
fn reject(request: &Request, code: wire::ExecutionErrorCode) -> Reply {
    match request {
        Request::Execute { command } => Reply::Rejected {
            channel: command.channel.clone(),
            identity: command.identity.clone(),
            code,
        },
        Request::Reconcile { channel, identity } => Reply::Rejected {
            channel: channel.clone(),
            identity: identity.clone(),
            code,
        },
        Request::OutcomeAck {
            channel,
            identity,
            fence,
            outcome_id,
            outcome_digest,
        } => Reply::OutcomeAckRejected {
            channel: channel.clone(),
            identity: identity.clone(),
            fence: fence.clone(),
            outcome_id: outcome_id.clone(),
            outcome_digest: outcome_digest.clone(),
            code,
        },
        Request::Renew { fence, .. } | Request::Cancel { fence } => Reply::ControlRejected {
            fence: fence.clone(),
            code,
        },
    }
}
