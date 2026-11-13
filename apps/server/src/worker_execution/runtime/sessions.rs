//! One persistent OS/ACP context. Turn completion never releases its reservation.
use super::*;
use crate::worker_execution::session_acp as engine;
use crate::worker_exports::session::{SessionFilesystem, WorkerSessionLease, WorkerTurnLease};
use tokio::sync::mpsc;
use wire::session::{
    self as session, SessionControllerMessage as Request, SessionWorkerMessage as Reply,
};
enum Open {
    Legacy(Box<session::SessionOpenCommand>),
    Broker(Box<session::broker::OpenBrokerSessionCommand>),
}
impl Open {
    fn command(&self) -> &session::SessionOpenCommand {
        match self {
            Self::Legacy(command) => command,
            Self::Broker(command) => &command.open,
        }
    }
}
pub(super) struct McpCut {
    pub scope: session::broker::BrokerScope,
    pub deadline: Instant,
    pub limits: session::broker::BrokerLimits,
    pub profile: String,
}
struct SessionState {
    lease: WorkerSessionLease,
    turn: Option<WorkerTurnLease>,
}
pub(super) struct SessionActive {
    command: session::SessionOpenCommand,
    receipt: session::SessionReceipt,
    checkout_root: PathBuf,
    connection: Connected,
    state: Mutex<SessionState>,
    filesystem: SessionFilesystem,
    deadline_ms: AtomicU64,
    stop: AtomicBool,
    group: Arc<process::Group>,
    sender: mpsc::Sender<session::TurnCommand>,
    finished: AtomicBool,
    stopped: AtomicBool,
    initialized: AtomicBool,
}
impl SessionActive {
    pub(super) fn wants_mcp(&self) -> bool {
        !self.command.definition.mcp.is_empty()
    }
    pub(super) fn client(&self) -> Result<crate::worker_client::broker::Client, Code> {
        self.connection
            .broker
            .clone()
            .ok_or(Code::UnsupportedProfile)
    }
    pub(super) fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.group.terminate();
    }
}
impl Runtime {
    pub async fn session_outbox(
        self: &Arc<Self>,
        connection: &Connected,
    ) -> Result<Vec<Reply>, Code> {
        let permit = self
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| Code::CapacityReached)?;
        let runtime = self.clone();
        let connection = connection.clone();
        tokio::time::timeout(
            Duration::from_secs(3),
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let store = runtime.store()?;
                let guard = || connection.authorize();
                let context = connection
                    .context(&runtime.config.scope, &guard)
                    .map_err(code)?;
                let mut replies = store
                    .session_outbox(&context)
                    .map_err(code)?
                    .into_iter()
                    .map(|event| Reply::Event { event })
                    .collect::<Vec<_>>();
                let active = runtime
                    .session
                    .lock()
                    .map_err(|_| Code::Unavailable)?
                    .clone();
                if let Some(active) = active
                    && active.connection.fence == connection.fence
                    && active.initialized.load(Ordering::Acquire)
                    && runtime.session_live(&active).is_ok()
                {
                    let state = active.state.lock().map_err(|_| Code::Unavailable)?;
                    if store
                        .validate_worker_session(&context, &state.lease)
                        .is_ok()
                    {
                        // A v5 tool may need consent before the agent emits
                        // any text. Recover a lost Submit receipt independently
                        // from that text, using only the original active lease.
                        if state.lease.broker_limits().is_some()
                            && let Some(turn) = &state.turn
                            && store
                                .validate_worker_turn(&context, &state.lease, turn)
                                .is_ok()
                        {
                            replies.insert(
                                0,
                                Reply::TurnAccepted {
                                    receipt: turn.receipt().clone(),
                                },
                            );
                        }
                        replies.insert(
                            0,
                            Reply::SessionReady {
                                receipt: active.receipt.clone(),
                            },
                        );
                    }
                }
                Ok(replies)
            }),
        )
        .await
        .map_err(|_| Code::Unavailable)?
        .map_err(|_| Code::Unavailable)?
    }
    fn session_live(&self, active: &SessionActive) -> Result<(), Code> {
        if self.closed.load(Ordering::Acquire)
            || active.stop.load(Ordering::Acquire)
            || self.milliseconds(Instant::now()) >= active.deadline_ms.load(Ordering::Acquire)
        {
            return Err(Code::LeaseExpired);
        }
        active.connection.authorize().map_err(code)
    }
    fn session_slot(&self, fence: &session::SessionFence) -> Result<Arc<SessionActive>, Code> {
        self.session
            .lock()
            .map_err(|_| Code::Unavailable)?
            .as_ref()
            .filter(|a| a.command.controller_session_id == fence.controller_session_id)
            .cloned()
            .ok_or(Code::StaleAttempt)
            .and_then(|a| {
                if &a.receipt.fence != fence {
                    return Err(Code::StaleAttempt);
                }
                Ok(a)
            })
    }
    /// Called with the state mutex held; old effective deadline is checked before
    /// publishing any extension. OS guardian independently latches old expiry.
    fn session_deadline(&self, active: &SessionActive, state: &SessionState) -> Result<(), Code> {
        self.session_live(active)?;
        let sampled = process::monotonic_ns().map_err(|_| Code::Unavailable)?;
        let sampled_instant = Instant::now();
        let mut remaining = state.lease.remaining().map_err(code)?;
        if let Some(turn) = &state.turn {
            remaining = remaining.min(turn.remaining().map_err(code)?);
        }
        let until = sampled_instant + remaining;
        active
            .group
            .renew(
                sampled
                    .checked_add(
                        u64::try_from(remaining.as_nanos()).map_err(|_| Code::Unavailable)?,
                    )
                    .ok_or(Code::Unavailable)?,
            )
            .map_err(|_| Code::LeaseExpired)?;
        self.session_live(active)?;
        active
            .deadline_ms
            .store(self.milliseconds(until), Ordering::Release);
        Ok(())
    }
    async fn session_sql<T: Send + 'static>(
        self: &Arc<Self>,
        active: &Arc<SessionActive>,
        operation: impl FnOnce(
            &Runtime,
            &SessionActive,
            &WorkerStore,
            &ExecutionChannel<'_>,
            &mut SessionState,
        ) -> Result<T, Code>
        + Send
        + 'static,
    ) -> Result<T, Code> {
        let permit = self
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| Code::CapacityReached)?;
        let runtime = self.clone();
        let active = active.clone();
        let failed = active.clone();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                runtime.session_live(&active)?;
                let store = runtime.store()?;
                let guard = || active.connection.authorize();
                let ctx = active
                    .connection
                    .context(&runtime.config.scope, &guard)
                    .map_err(code)?;
                let mut state = active.state.lock().map_err(|_| Code::Unavailable)?;
                runtime.session_live(&active)?;
                operation(&runtime, &active, &store, &ctx, &mut state)
            }),
        )
        .await;
        match result {
            Ok(Ok(value)) => value,
            _ => {
                failed.stop();
                Err(Code::Unavailable)
            }
        }
    }
    async fn session_validate(self: &Arc<Self>, active: &Arc<SessionActive>) -> Result<(), Code> {
        let permit = tokio::time::timeout(Duration::from_secs(2), self.fs.clone().acquire_owned())
            .await
            .map_err(|_| Code::Unavailable)?
            .map_err(|_| Code::Unavailable)?;
        let runtime = self.clone();
        let active = active.clone();
        tokio::time::timeout(
            Duration::from_secs(3),
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                runtime.session_live(&active)?;
                {
                    let s = runtime.store()?;
                    let guard = || active.connection.authorize();
                    let ctx = active
                        .connection
                        .context(&runtime.config.scope, &guard)
                        .map_err(code)?;
                    let state = active.state.lock().map_err(|_| Code::Unavailable)?;
                    s.validate_worker_session(&ctx, &state.lease)
                        .map_err(code)?;
                    if let Some(turn) = &state.turn {
                        s.validate_worker_turn(&ctx, &state.lease, turn)
                            .map_err(code)?;
                    }
                }
                active.filesystem.validate().map_err(code)?;
                runtime.session_live(&active)?;
                let s = runtime.store()?;
                let guard = || active.connection.authorize();
                let ctx = active
                    .connection
                    .context(&runtime.config.scope, &guard)
                    .map_err(code)?;
                let state = active.state.lock().map_err(|_| Code::Unavailable)?;
                s.validate_worker_session(&ctx, &state.lease)
                    .map_err(code)?;
                if let Some(turn) = &state.turn {
                    s.validate_worker_turn(&ctx, &state.lease, turn)
                        .map_err(code)?;
                }
                runtime.session_live(&active)
            }),
        )
        .await
        .map_err(|_| Code::Unavailable)?
        .map_err(|_| Code::Unavailable)?
    }
    async fn open_live_session(
        self: &Arc<Self>,
        connection: Connected,
        open: Open,
    ) -> Result<Reply, Code> {
        let command = open.command();
        if command.channel != connection.fence {
            return Err(Code::StaleChannel);
        }
        let harness = self
            .config
            .harnesses
            .get(&command.definition.harness)
            .cloned()
            .ok_or(Code::UnsupportedProfile)?;
        let permit = tokio::time::timeout(Duration::from_secs(3), self.fs.clone().acquire_owned())
            .await
            .map_err(|_| Code::Unavailable)?
            .map_err(|_| Code::Unavailable)?;
        let runtime = self.clone();
        // This job owns admission and actor registration even if the socket waiter
        // disappears. No accepted launch permission is dropped between those cuts.
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            connection.authorize().map_err(code)?;
            let store = runtime.store()?;
            let command = open.command();
            let principal = local::ExportPrincipal {
                kind: local::PrincipalKind::Human,
                id: command.descriptor.grant.principal.id.clone(),
            };
            let validated = store
                .candidate(
                    &runtime.config.scope,
                    &command.descriptor.export.id,
                    &principal,
                    local::ExportPermission::Execute,
                )
                .map_err(code)?
                .validate_filesystem()
                .map_err(code)?;
            validated
                .reject_private_overlap(&runtime.protected)
                .map_err(code)?;
            let guard = || connection.authorize();
            let context = connection
                .context(&runtime.config.scope, &guard)
                .map_err(code)?;
            let admission = match &open {
                Open::Legacy(command) => store.open_session(&context, command, validated),
                Open::Broker(command) => store.open_broker_session(&context, command, validated),
            }
            .map_err(code)?;
            let command = match open {
                Open::Legacy(command) => *command,
                Open::Broker(command) => command.open,
            };
            if let Some(lease) = admission.lease {
                let sampled = Instant::now();
                let remaining = lease.remaining().unwrap_or_default();
                let (sender, receiver) = mpsc::channel(1);
                let active = Arc::new(SessionActive {
                    filesystem: lease.filesystem_guard(),
                    checkout_root: lease.root().to_owned(),
                    receipt: admission.receipt.clone(),
                    command,
                    connection: connection.clone(),
                    state: Mutex::new(SessionState { lease, turn: None }),
                    deadline_ms: AtomicU64::new(runtime.milliseconds(sampled + remaining)),
                    stop: AtomicBool::new(false),
                    group: Arc::new(process::Group::default()),
                    sender,
                    finished: AtomicBool::new(false),
                    stopped: AtomicBool::new(false),
                    initialized: AtomicBool::new(false),
                });
                let registered = runtime
                    .session
                    .lock()
                    .map(|mut slot| {
                        if slot
                            .as_ref()
                            .is_some_and(|a| !a.stopped.load(Ordering::Acquire))
                        {
                            false
                        } else {
                            *slot = Some(active.clone());
                            true
                        }
                    })
                    .unwrap_or(false);
                if !registered {
                    let state = active.state.lock().map_err(|_| Code::Unavailable)?;
                    let proof =
                        process::StopProof::never_spawned_session(state.lease.fence().clone());
                    store
                        .finish_stopped_session(
                            &runtime.config.scope,
                            &active.command.identity,
                            state.lease.fence(),
                            |f| proof.attest_session(f),
                        )
                        .map_err(code)?;
                    return Err(Code::CapacityReached);
                }
                let task = runtime.clone();
                tokio::spawn(async move {
                    task.session_actor(active, harness, receiver).await;
                });
            }
            Ok(Reply::SessionAccepted {
                receipt: admission.receipt,
            })
        })
        .await
        .map_err(|_| Code::Unavailable)?
    }
    async fn submit_session_turn(
        self: &Arc<Self>,
        connection: &Connected,
        command: session::TurnCommand,
    ) -> Result<Reply, Code> {
        if command.session.channel != connection.fence {
            return Err(Code::StaleChannel);
        }
        let permit = self
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| Code::CapacityReached)?;
        let runtime = self.clone();
        let connection = connection.clone();
        tokio::time::timeout(
            Duration::from_secs(5),
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let store = runtime.store()?;
                let guard = || connection.authorize();
                let context = connection
                    .context(&runtime.config.scope, &guard)
                    .map_err(code)?;
                if let Some(record) = store
                    .lookup_session_turn(&context, &command)
                    .map_err(code)?
                {
                    return Ok(Reply::TurnAccepted {
                        receipt: record.receipt,
                    });
                }
                let active = runtime.session_slot(&command.session)?;
                runtime.session_live(&active)?;
                let mut state = active.state.lock().map_err(|_| Code::Unavailable)?;
                let admission = store
                    .admit_session_turn(&context, &mut state.lease, &command)
                    .map_err(code)?;
                if let Some(turn) = admission.lease {
                    state.turn = Some(turn);
                    if runtime.session_deadline(&active, &state).is_err()
                        || active.sender.try_send(command).is_err()
                    {
                        active.stop();
                        return Err(Code::Unavailable);
                    }
                }
                Ok(Reply::TurnAccepted {
                    receipt: admission.receipt,
                })
            }),
        )
        .await
        .map_err(|_| Code::Unavailable)?
        .map_err(|_| Code::Unavailable)?
    }
    async fn session_close(
        self: &Arc<Self>,
        connection: &Connected,
        fence: session::SessionFence,
        turn: Option<session::TurnFence>,
    ) -> Result<Option<Reply>, Code> {
        if fence.channel != connection.fence {
            return Err(Code::StaleChannel);
        }
        let permit = self
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| Code::CapacityReached)?;
        let runtime = self.clone();
        let connection = connection.clone();
        tokio::time::timeout(
            Duration::from_secs(5),
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let store = runtime.store()?;
                let guard = || connection.authorize();
                let context = connection
                    .context(&runtime.config.scope, &guard)
                    .map_err(code)?;
                // Store checks terminal-turn identity before cancellation can affect a later turn.
                if store
                    .request_session_close(&context, &fence, turn.as_ref())
                    .map_err(code)?
                    && let Ok(active) = runtime.session_slot(&fence)
                {
                    active.stop();
                }
                Ok(Some(match turn {
                    Some(turn) => Reply::CancelRequested { turn },
                    None => Reply::CloseRequested { session: fence },
                }))
            }),
        )
        .await
        .map_err(|_| Code::Unavailable)?
        .map_err(|_| Code::Unavailable)?
    }
    fn session_hooks(self: &Arc<Self>, active: &Arc<SessionActive>) -> engine::Hooks {
        let r = self.clone();
        let a = active.clone();
        let guard = Arc::new(move || {
            r.session_live(&a).map_err(|_| "session_guard_closed")?;
            let ms = a.deadline_ms.load(Ordering::Acquire);
            Ok(tokio::time::Instant::from_std(
                r.origin + Duration::from_millis(ms),
            ))
        });
        let r = self.clone();
        let a = active.clone();
        let before_prompt = Arc::new(move |intent: engine::PromptIntent| {
            let r = r.clone();
            let a = a.clone();
            Box::pin(async move {
                r.session_sql(&a, move |_, _, store, ctx, state| {
                    let turn = state
                        .turn
                        .as_ref()
                        .filter(|t| t.fence().turn_id == intent.turn.turn_id)
                        .ok_or(Code::StaleAttempt)?;
                    store
                        .turn_write_intent(ctx, &state.lease, turn)
                        .map_err(code)
                })
                .await
                .map_err(|_| "session_intent_failed")
            }) as engine::HookFuture
        });
        let r = self.clone();
        let a = active.clone();
        let stream = Arc::new(move |chunk: engine::StreamChunk| {
            let r = r.clone();
            let a = a.clone();
            Box::pin(async move {
                r.session_sql(&a, move |_, _active, store, ctx, state| {
                    let turn = state
                        .turn
                        .as_ref()
                        .filter(|t| t.fence().turn_id == chunk.turn_id)
                        .ok_or(Code::StaleAttempt)?;
                    let kind = match chunk.kind {
                        engine::StreamKind::Text => session::TextKind::Text,
                        engine::StreamKind::Reasoning => session::TextKind::Reasoning,
                    };
                    store
                        .append_session_text(ctx, &mut state.lease, turn, kind, &chunk.text)
                        .map_err(code)?;
                    #[cfg(all(test, target_os = "linux"))]
                    super::tests::fault_checkpoint("running", &_active.group);
                    Ok(())
                })
                .await
                .map_err(|_| "session_stream_failed")
            }) as engine::HookFuture
        });
        engine::Hooks {
            guard,
            before_prompt,
            stream,
        }
    }
    async fn session_prepare(
        self: &Arc<Self>,
        active: &Arc<SessionActive>,
        harness: LocalHarness,
        peer: Option<Arc<super::mcp::Peer>>,
    ) -> Result<tokio::process::Command, Code> {
        let permit = tokio::time::timeout(Duration::from_secs(3), self.fs.clone().acquire_owned())
            .await
            .map_err(|_| Code::Unavailable)?
            .map_err(|_| Code::Unavailable)?;
        let r = self.clone();
        let a = active.clone();
        tokio::time::timeout(
            Duration::from_secs(3),
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                r.session_live(&a)?;
                let (root, metadata) = {
                    let state = a.state.lock().map_err(|_| Code::Unavailable)?;
                    (
                        state.lease.root().to_owned(),
                        state.lease.sandbox_metadata_roots().map_err(code)?,
                    )
                };
                let command = sandbox_command_with_proxy(
                    &r.config,
                    &harness,
                    &root,
                    &metadata,
                    &a.command.descriptor.grant.principal.id,
                    peer.as_ref().map_or(&[], |peer| peer.paths()),
                )
                .map_err(|_| Code::UnsupportedProfile)?;
                r.session_live(&a)?;
                Ok(command)
            }),
        )
        .await
        .map_err(|_| Code::Unavailable)?
        .map_err(|_| Code::Unavailable)?
    }
    async fn session_record_stop(
        self: &Arc<Self>,
        active: &Arc<SessionActive>,
        proof: process::StopProof,
    ) -> bool {
        let r = self.clone();
        let a = active.clone();
        let persisted = matches!(
            tokio::task::spawn_blocking(move || {
                let store = r.store()?;
                let state = a.state.lock().map_err(|_| Code::Unavailable)?;
                store
                    .finish_stopped_session(
                        &r.config.scope,
                        &a.command.identity,
                        state.lease.fence(),
                        |f| proof.attest_session(f),
                    )
                    .map_err(code)?;
                Ok::<_, Code>(())
            })
            .await,
            Ok(Ok(()))
        );
        // Only actual OS stop permits replacing the in-memory process slot. SQL
        // reservation remains quarantined if persistence failed.
        active.stopped.store(true, Ordering::Release);
        persisted
    }
    async fn session_actor(
        self: Arc<Self>,
        active: Arc<SessionActive>,
        harness: LocalHarness,
        mut receiver: mpsc::Receiver<session::TurnCommand>,
    ) {
        let guards = self.session_guards(&active);
        let mut owned = None;
        let run: Result<(), Code> = async {
            self.session_validate(&active).await?;
            self.session_sql(&active, |_, _, store, context, state| {
                store
                    .session_spawn_intent(context, &state.lease)
                    .map_err(code)
            })
            .await?;
            let peer = super::mcp::Peer::start(&self, &active).await?;
            let command = self.session_prepare(&active, harness, peer.clone()).await?;
            self.session_validate(&active).await?;
            let owner = process::SessionOwner {
                identity: active.command.identity.clone(),
                fence: active.receipt.fence.clone(),
                group: active.group.clone(),
            };
            let runtime = self.clone();
            let spawning = active.clone();
            owned = Some(
                tokio::task::spawn_blocking(move || {
                    runtime.session_live(&spawning)?;
                    process::Owned::spawn_session(
                        &runtime.platform,
                        &runtime.config,
                        command,
                        owner,
                        || {
                            let state = spawning.state.lock().map_err(|_| local::Error::Denied)?;
                            Ok((state.lease.remaining()?, state.lease.budget_remaining()?))
                        },
                    )
                    .map_err(|_| Code::Unavailable)
                })
                .await
                .map_err(|_| Code::Unavailable)??,
            );
            let child = owned.as_mut().expect("session process owner");
            child
                .ready(&active.connection)
                .await
                .map_err(|_| Code::Unavailable)?;
            self.session_validate(&active).await?;
            child.release().map_err(|_| Code::Unavailable)?;
            let engine = engine::Engine::start(
                child.child.stdin.take().ok_or(Code::Unavailable)?,
                child.child.stdout.take().ok_or(Code::Unavailable)?,
                engine::EngineConfig {
                    cwd: active.checkout_root.clone(),
                    definition: active.command.definition.clone(),
                    tools: peer.as_ref().map(|peer| peer.tools()),
                    max_session_seconds: active.command.budgets.max_seconds,
                    max_output_bytes: active.command.budgets.max_output_bytes,
                    max_turns: active.command.budgets.max_turns,
                    idle_timeout_ms: active.command.budgets.idle_timeout_ms,
                },
                self.session_hooks(&active),
            )
            .await
            .map_err(|_| Code::UnsupportedProfile)?;
            if engine.ready().model != active.command.definition.model
                || engine.ready().mode != active.command.definition.mode
            {
                return Err(Code::UnsupportedProfile);
            }
            self.session_sql(&active, |runtime, active, store, context, state| {
                store
                    .session_ready(context, &mut state.lease)
                    .map_err(code)?;
                runtime.session_deadline(active, state)?;
                active.initialized.store(true, Ordering::Release);
                Ok(())
            })
            .await?;
            loop {
                let command = tokio::select! {
                    biased;
                    _ = engine.closed() => return Err(Code::Unavailable),
                    command = receiver.recv() => command.ok_or(Code::Unavailable)?,
                };
                self.session_validate(&active).await?;
                let result = engine
                    .turn(engine::TurnSpec {
                        turn_id: command.identity.turn_id.clone(),
                        text: command.input,
                        max_seconds: command.budgets.max_seconds,
                        max_output_bytes: command.budgets.max_output_bytes,
                    })
                    .await
                    .map_err(|_| Code::Unavailable)?;
                self.session_sql(&active, move |runtime, active, store, context, state| {
                    let turn = state
                        .turn
                        .as_mut()
                        .filter(|turn| turn.fence().turn_id == result.turn_id)
                        .ok_or(Code::StaleAttempt)?;
                    store
                        .complete_session_turn(context, &mut state.lease, turn, result.status)
                        .map_err(code)?;
                    state.turn = None;
                    runtime.session_deadline(active, state)
                })
                .await?;
            }
        }
        .await;
        #[cfg(test)]
        if let Err(error) = run {
            eprintln!("worker session actor stopped: {error:?}");
        }
        #[cfg(not(test))]
        let _ = run;
        // Failure always closes context; never fabricates a successful turn.
        active.stop();
        if let Some(mut child) = owned {
            if let Ok(proof) = child.stop().await {
                if self.session_record_stop(&active, proof).await {
                    let _ = child.finalized();
                }
                let _ = child.reap().await;
            }
        } else {
            self.session_record_stop(
                &active,
                process::StopProof::never_spawned_session(active.receipt.fence.clone()),
            )
            .await;
        }
        active.finished.store(true, Ordering::Release);
        guards.0.abort();
        guards.1.abort();
    }
    pub(super) fn disconnect_session(&self, connection: &Connected) {
        if let Ok(slot) = self.session.lock()
            && let Some(active) = slot.as_ref()
            && active.connection.fence == connection.fence
        {
            active.stop();
        }
    }
    pub(super) async fn shutdown_session(&self) {
        let active = self.session.lock().ok().and_then(|s| s.clone());
        if let Some(active) = active {
            active.stop();
            let _ = tokio::time::timeout(Duration::from_secs(10), async {
                while !active.finished.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await;
        }
    }
}
impl Runtime {
    fn session_guards(
        self: &Arc<Self>,
        active: &Arc<SessionActive>,
    ) -> (tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>) {
        let runtime = self.clone();
        let a = active.clone();
        let clock = tokio::spawn(async move {
            while !a.finished.load(Ordering::Acquire) {
                if a.group.guard_exited() || runtime.session_live(&a).is_err() {
                    a.stop();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        let runtime = self.clone();
        let a = active.clone();
        let authority = tokio::spawn(async move {
            while !a.finished.load(Ordering::Acquire) {
                if runtime.session_validate(&a).await.is_err() {
                    a.stop();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
        (clock, authority)
    }
    async fn accept_open(
        self: &Arc<Self>,
        connection: &Connected,
        open: Open,
    ) -> Result<Option<Reply>, Code> {
        let permit = self
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| Code::CapacityReached)?;
        let runtime = self.clone();
        let connection = connection.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _permit = permit;
            let result = runtime.open_live_session(connection, open).await.map(Some);
            let _ = send.send(result);
        });
        tokio::time::timeout(Duration::from_secs(8), receive)
            .await
            .map_err(|_| Code::Unavailable)?
            .map_err(|_| Code::Unavailable)?
    }
    pub(crate) async fn handle_broker_open(
        self: &Arc<Self>,
        connection: &Connected,
        command: session::broker::OpenBrokerSessionCommand,
    ) -> Result<Option<Reply>, Code> {
        if !self.supports_sessions() {
            return Err(Code::UnsupportedProfile);
        }
        connection.authorize().map_err(code)?;
        let client = connection.broker.as_ref().ok_or(Code::UnsupportedProfile)?;
        command
            .validate_for_channel(&connection.fence, client.limits())
            .map_err(|_| Code::IdentityConflict)?;
        self.accept_open(connection, Open::Broker(Box::new(command)))
            .await
    }
    pub(super) fn mcp_live(&self, active: &SessionActive) -> Result<(), Code> {
        self.session_live(active)
    }
    pub(super) async fn mcp_cut(
        self: &Arc<Self>,
        active: &Arc<SessionActive>,
        call: bool,
        expected: Option<session::broker::BrokerScope>,
    ) -> Result<McpCut, Code> {
        self.session_validate(active).await?;
        let cut = self
            .session_sql(active, move |_, active, store, context, state| {
                store
                    .validate_worker_session(context, &state.lease)
                    .map_err(code)?;
                let limits = state
                    .lease
                    .broker_limits()
                    .cloned()
                    .ok_or(Code::UnsupportedProfile)?;
                let sampled = Instant::now();
                let mut remaining = state
                    .lease
                    .budget_remaining()
                    .map_err(code)?
                    .min(Duration::from_secs(150));
                let scope = if call {
                    let turn = state.turn.as_ref().ok_or(Code::AuthorityRevoked)?;
                    store
                        .validate_worker_turn(context, &state.lease, turn)
                        .map_err(code)?;
                    remaining = remaining.min(turn.budget_remaining().map_err(code)?);
                    session::broker::BrokerScope::Turn {
                        turn: turn.fence().clone(),
                    }
                } else {
                    session::broker::BrokerScope::Catalog {
                        session: state.lease.fence().clone(),
                    }
                };
                if expected.as_ref().is_some_and(|expected| expected != &scope) {
                    return Err(Code::StaleAttempt);
                }
                Ok(McpCut {
                    scope,
                    deadline: sampled + remaining,
                    limits,
                    profile: active.command.profile.digest.clone(),
                })
            })
            .await?;
        self.session_validate(active).await?;
        if Instant::now() >= cut.deadline {
            return Err(Code::LeaseExpired);
        }
        Ok(cut)
    }
    pub async fn handle_session(
        self: &Arc<Self>,
        connection: &Connected,
        request: Request,
    ) -> Result<Option<Reply>, Code> {
        if !self.supports_sessions() {
            return Err(Code::UnsupportedProfile);
        }
        request.validate().map_err(|_| Code::IdentityConflict)?;
        connection.authorize().map_err(code)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(Code::StaleChannel);
        }
        if let Request::OpenSession { command } = request {
            return self.accept_open(connection, Open::Legacy(command)).await;
        }
        if let Request::SubmitTurn { command } = request {
            return self
                .submit_session_turn(connection, *command)
                .await
                .map(Some);
        }
        match request {
            Request::RenewSession {
                session: fence,
                sequence,
                lease_duration_ms,
            } => {
                if fence.channel != connection.fence {
                    return Err(Code::StaleChannel);
                }
                let active = self.session_slot(&fence)?;
                self.session_sql(&active, move |r, a, s, c, state| {
                    s.renew_worker_session(c, &mut state.lease, sequence, lease_duration_ms)
                        .map_err(code)?;
                    r.session_deadline(a, state)?;
                    Ok(Some(Reply::SessionRenewed {
                        session: fence,
                        sequence,
                        lease_duration_ms,
                    }))
                })
                .await
            }
            Request::RenewTurn {
                turn: fence,
                sequence,
                lease_duration_ms,
            } => {
                if fence.session.channel != connection.fence {
                    return Err(Code::StaleChannel);
                }
                let active = self.session_slot(&fence.session)?;
                self.session_sql(&active, move |r, a, s, c, state| {
                    let turn = state
                        .turn
                        .as_mut()
                        .filter(|t| t.fence() == &fence)
                        .ok_or(Code::StaleAttempt)?;
                    s.renew_worker_turn(c, &state.lease, turn, sequence, lease_duration_ms)
                        .map_err(code)?;
                    r.session_deadline(a, state)?;
                    Ok(Some(Reply::TurnRenewed {
                        turn: fence,
                        sequence,
                        lease_duration_ms,
                    }))
                })
                .await
            }
            Request::CloseSession { session: fence } => {
                self.session_close(connection, fence, None).await
            }
            Request::CancelTurn { turn } => {
                self.session_close(connection, turn.session.clone(), Some(turn))
                    .await
            }
            other => self.session_read(connection, other).await,
        }
    }
    async fn session_read(
        self: &Arc<Self>,
        connection: &Connected,
        request: Request,
    ) -> Result<Option<Reply>, Code> {
        let permit = self
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| Code::CapacityReached)?;
        let runtime = self.clone();
        let connection = connection.clone();
        tokio::time::timeout(
            Duration::from_secs(5),
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let store = runtime.store()?;
                let guard = || connection.authorize();
                let ctx = connection
                    .context(&runtime.config.scope, &guard)
                    .map_err(code)?;
                Ok(Some(match request {
                    Request::ReconcileSession { channel, identity } => {
                        if channel != connection.fence {
                            return Err(Code::StaleChannel);
                        }
                        let record = store
                            .reconcile_worker_session(&ctx, &identity)
                            .map_err(code)?;
                        Reply::SessionReconciled {
                            channel,
                            identity,
                            record,
                        }
                    }
                    Request::ReconcileTurn {
                        channel,
                        session,
                        identity,
                    } => {
                        if channel != connection.fence {
                            return Err(Code::StaleChannel);
                        }
                        let record = store
                            .reconcile_worker_turn(&ctx, &session, &identity)
                            .map_err(code)?;
                        Reply::TurnReconciled {
                            channel,
                            session,
                            identity,
                            record,
                        }
                    }
                    Request::ReplayStream {
                        channel,
                        session,
                        after_sequence,
                        after_digest,
                        limit,
                    } => {
                        if channel != connection.fence {
                            return Err(Code::StaleChannel);
                        }
                        let (through_sequence, events, more) = store
                            .replay_session_stream(
                                &ctx,
                                &session,
                                after_sequence,
                                after_digest.as_deref(),
                                limit as usize,
                            )
                            .map_err(code)?;
                        Reply::StreamReplayed {
                            channel,
                            session,
                            after_sequence,
                            after_digest,
                            limit,
                            through_sequence,
                            events,
                            more,
                        }
                    }
                    Request::AckStream {
                        channel,
                        session,
                        through_sequence,
                        event_digest,
                    } => {
                        if channel != connection.fence {
                            return Err(Code::StaleChannel);
                        }
                        store
                            .acknowledge_session_stream(
                                &ctx,
                                &session,
                                through_sequence,
                                &event_digest,
                            )
                            .map_err(code)?;
                        Reply::StreamAcknowledged {
                            channel,
                            session,
                            through_sequence,
                            event_digest,
                        }
                    }
                    _ => return Err(Code::UnsupportedProfile),
                }))
            }),
        )
        .await
        .map_err(|_| Code::Unavailable)?
        .map_err(|_| Code::Unavailable)?
    }
}
