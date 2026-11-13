use super::*;
impl Runtime {
    async fn transition(self: &Arc<Self>, active: &Arc<Active>, running: bool) -> Result<(), Code> {
        let runtime = self.clone();
        let active = active.clone();
        tokio::task::spawn_blocking(move || {
            runtime.current(&active)?;
            let store = runtime.store()?;
            let guard = || active.connection.authorize();
            let context = active
                .connection
                .context(&runtime.config.scope, &guard)
                .map_err(code)?;
            let lease = active.lease.lock().map_err(|_| Code::Unavailable)?;
            if running {
                store.execution_running(&context, &lease).map_err(code)
            } else {
                store.execution_start_intent(&context, &lease).map_err(code)
            }
        })
        .await
        .map_err(|_| Code::Unavailable)?
    }
    async fn prepare_command(
        self: &Arc<Self>,
        active: &Arc<Active>,
        harness: LocalHarness,
    ) -> Result<tokio::process::Command, Code> {
        let permit = self
            .fs
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Code::Unavailable)?;
        let runtime = self.clone();
        let active = active.clone();
        tokio::time::timeout(
            Duration::from_secs(3),
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                runtime.current(&active)?;
                let (root, metadata) = {
                    let lease = active.lease.lock().map_err(|_| Code::Unavailable)?;
                    (
                        lease.root().to_owned(),
                        lease.sandbox_metadata_roots().map_err(code)?,
                    )
                };
                let command = sandbox_command(
                    &runtime.config,
                    &harness,
                    &root,
                    &metadata,
                    &active.command.descriptor.grant.principal.id,
                )
                .map_err(|_| Code::UnsupportedProfile)?;
                runtime.current(&active)?;
                Ok::<_, Code>(command)
            }),
        )
        .await
        .map_err(|_| Code::Unavailable)?
        .map_err(|_| Code::Unavailable)?
    }
    async fn record_stopped(
        self: &Arc<Self>,
        active: &Arc<Active>,
        proof: process::StopProof,
        result: acp::ResultText,
    ) -> bool {
        active.stopped.store(true, Ordering::Release);
        let runtime = self.clone();
        let active = active.clone();
        matches!(
            tokio::task::spawn_blocking(move || -> Result<(), Code> {
                let store = runtime.store()?;
                let guard = || active.connection.authorize();
                // Negative recovery may run after the channel was permanently lost.
                // Building a borrowed context never restores a positive launch lease.
                let context = ExecutionChannel::verify(
                    &runtime.config.scope,
                    &active.connection.fence,
                    &guard,
                );
                let lease = active.lease.lock().map_err(|_| Code::Unavailable)?;
                let mut outcome = wire::ExecutionOutcome {
                    outcome_id: format!("outcome-{}", uuid::Uuid::now_v7()),
                    digest: "0".repeat(64),
                    status: if proof.recovered() {
                        wire::ExecutionTerminalStatus::Unknown
                    } else {
                        result.status
                    },
                    text: result.text,
                };
                outcome.digest = crate::worker_execution::outcome_digest(&outcome)
                    .map_err(|_| Code::Unavailable)?;
                // If fresh positive completion cannot be established, only the
                // stop-attested negative API may publish Unknown and release quota.
                let finished = match context {
                    Ok(context) => store.finish_execution(&context, &lease, &outcome),
                    Err(_) => Err(local::Error::Denied),
                };
                if finished.is_ok() {
                    store
                        .confirm_execution_stopped(
                            &runtime.config.scope,
                            &active.command.identity,
                            lease.fence(),
                            |fence| proof.attest(fence),
                        )
                        .map_err(code)?;
                } else {
                    store
                        .finish_stopped_execution(
                            &runtime.config.scope,
                            &active.command.identity,
                            lease.fence(),
                            |fence| proof.attest(fence),
                        )
                        .map_err(code)?;
                }
                Ok(())
            })
            .await,
            Ok(Ok(()))
        )
    }
    pub(super) async fn actor(self: Arc<Self>, active: Arc<Active>, harness: LocalHarness) {
        let guards = self.start_guards(&active);
        let mut owned = None;
        let mut result = acp::ResultText {
            status: wire::ExecutionTerminalStatus::Unknown,
            text: "Execution did not produce a confirmed result".into(),
        };
        let setup = async {
            self.validate_active(&active).await?;
            self.transition(&active, false).await?;
            let command = self.prepare_command(&active, harness).await?;
            self.validate_active(&active).await?;
            // Owned is retained across every bootstrap error: a failed handshake
            // never discards the process/namespace ownership cleanup obligation.
            owned = Some(
                process::Owned::spawn(&self.platform, &self.config, command, &active)
                    .map_err(|_| Code::Unavailable)?,
            );
            let child = owned.as_mut().expect("created child owner");
            child
                .ready(&active.connection)
                .await
                .map_err(|_| Code::Unavailable)?;
            self.validate_active(&active).await?;
            self.transition(&active, true).await?;
            self.validate_active(&active).await?;
            child.release().map_err(|_| Code::Unavailable)?;
            result = acp::conversation(&self, &active, &mut child.child).await;
            Ok::<_, Code>(())
        }
        .await;
        if let Err(error) = setup {
            result = acp::ResultText {
                status: wire::ExecutionTerminalStatus::Unknown,
                text: format!(
                    "Execution stopped before a complete result ({error:?}); effects may require review."
                ),
            };
        }
        if let Some(mut child) = owned {
            match child.stop().await {
                Ok(proof) => {
                    if self.record_stopped(&active, proof, result).await {
                        let _ = child.finalized();
                    }
                    let _ = child.reap().await;
                }
                Err(_) => {
                    // No stop proof, no release, no terminal wire delivery. Guard
                    // fallback/restart reconciliation must retain quarantine.
                    active.stop();
                }
            }
        } else {
            // This path never called a successful process spawn. No user process
            // could have been launched, even if start-intent was durable.
            let fence = active.lease.lock().ok().map(|lease| lease.fence().clone());
            if let Some(fence) = fence {
                let _ = self
                    .record_stopped(&active, process::StopProof::never_spawned(fence), result)
                    .await;
            }
        }
        active.stop();
        active.finished.store(true, Ordering::Release);
        active.done.notify_waiters();
        guards.0.abort();
        guards.1.abort();
    }
}
