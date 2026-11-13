use super::*;

#[derive(Clone)]
pub(super) enum Cut {
    Open,
    Claim,
    Submit(String),
    Renew,
    RenewTurn(String),
    ConfirmTerminal(String),
}
pub(super) enum Output {
    Open(RemoteOpen),
    Request(Request),
    Claim(Option<wire::TurnCommand>),
    Nothing,
}
pub(super) async fn positive(
    service: &Arc<Service>,
    actor: &Arc<Actor>,
    cut: Cut,
) -> Result<Output> {
    actor.running().map_err(|_| Error::Unauthorized)?;
    let snapshot = service.snapshot(&actor.id).await?;
    let proof = service.describe(&snapshot, &actor.transport).await?;
    let service = service.clone();
    let context = service.context.clone();
    let actor = actor.clone();
    #[cfg(test)]
    let stage = match &cut {
        Cut::Open => "open",
        Cut::Claim => "claim",
        Cut::Submit(_) => "submit",
        Cut::Renew => "renew_session",
        Cut::RenewTurn(_) => "renew_turn",
        Cut::ConfirmTerminal(_) => "confirm_terminal",
    };
    context
        .authority(move |ctx, store, policy| {
            ctx.running()?;
            actor.running()?;
            store.remote_binding_transaction(false, |store| {
                let authorize = |store: &Store, snapshot: &RemoteSessionSnapshot| {
                    ctx.running()?;
                    actor.authorize(snapshot)?;
                    service.source(store, snapshot, &actor.transport, false)?;
                    ctx.caller(store, &snapshot.owner, &snapshot.device, policy)
                };
                let scope = mcp::scope(&actor)?;
                let runtime = scope
                    .as_ref()
                    .map(|scope| scope.runtime(store, &service.config))
                    .unwrap_or_else(|| store.remote_session_runtime());
                let output = match cut {
                    Cut::Open => runtime
                        .dispatch_remote_session_open_in_transaction(
                            &actor.id,
                            proof,
                            now()?,
                            authorize,
                        )
                        .map(Output::Open),
                    Cut::Claim => runtime
                        .claim_remote_session_turn_in_transaction(
                            &actor.id,
                            proof,
                            now()?,
                            authorize,
                        )
                        .map(Output::Claim),
                    Cut::Submit(turn) => runtime
                        .dispatch_remote_session_turn_in_transaction(
                            &actor.id,
                            &turn,
                            proof,
                            now()?,
                            authorize,
                        )
                        .map(|command| {
                            Output::Request(Request::SubmitTurn {
                                command: Box::new(command),
                            })
                        }),
                    Cut::Renew => runtime
                        .renew_remote_session_in_transaction(&actor.id, proof, now()?, authorize)
                        .map(Output::Request),
                    Cut::RenewTurn(turn) => runtime
                        .renew_remote_session_turn_in_transaction(
                            &actor.id,
                            &turn,
                            proof,
                            now()?,
                            authorize,
                        )
                        .map(|message| message.map(Output::Request).unwrap_or(Output::Nothing)),
                    Cut::ConfirmTerminal(turn) => {
                        // Reuse the full fresh observation/authority cut. Only
                        // a durable canonical terminal can select None. If the
                        // turn is still active, reject inside this transaction
                        // so even its proposed renewal sequence rolls back.
                        match runtime.renew_remote_session_turn_in_transaction(
                            &actor.id,
                            &turn,
                            proof,
                            now()?,
                            authorize,
                        )? {
                            None => Ok(Output::Nothing),
                            Some(_) => {
                                Err(StoreError::Conflict("Remote turn is not terminal".into()))
                            }
                        }
                    }
                }?;
                actor.running()?;
                Ok(output)
            })
        })
        .await
        .inspect_err(|_error| {
            #[cfg(test)]
            eprintln!("remote session positive cut failed stage={stage} class={_error:?}");
        })
}
async fn exchange(service: &Service, actor: &Actor, message: Request) -> Result<Reply> {
    service.context.running().map_err(|_| Error::Unauthorized)?;
    actor.running().map_err(|_| Error::Unauthorized)?;
    #[cfg(test)]
    crate::worker_client::session_faults::before_renew(actor.transport.host_id(), &message).await;
    let reply = actor
        .transport
        .request(message.clone())
        .await
        .map_err(|_| Error::Uncertain)?;
    reply
        .validate_for_request(&message)
        .map_err(|_| Error::Conflict)?;
    Ok(reply)
}
async fn send(service: &Service, actor: &Actor, message: Request) -> Result<Reply> {
    let reply = exchange(service, actor, message).await?;
    if matches!(reply, Reply::Rejected { .. }) {
        return Err(Error::Conflict);
    }
    Ok(reply)
}
async fn renew_turn(service: &Arc<Service>, actor: &Arc<Actor>, message: Request) -> Result<()> {
    let Request::RenewTurn { turn, .. } = &message else {
        return Err(Error::Conflict);
    };
    let turn_id = turn.turn_id.clone();
    match exchange(service, actor, message).await? {
        Reply::TurnRenewed { .. } => Ok(()),
        Reply::Rejected {
            code: dunlin_relay_protocol::worker::ExecutionErrorCode::StaleAttempt,
            ..
        } => {
            // The source may have completed the original turn after our SQL
            // cut. Its refusal is not a renewal or an execution capability.
            // Import only the authenticated original stream, then require the
            // exact turn's durable terminal and fresh live v4/v5 authority.
            let snapshot = service.snapshot(&actor.id).await?;
            events::replay(service, &snapshot, &actor.transport).await?;
            if !matches!(
                positive(service, actor, Cut::ConfirmTerminal(turn_id)).await?,
                Output::Nothing
            ) {
                return Err(Error::Conflict);
            }
            #[cfg(test)]
            eprintln!(
                "remote turn renewal reconciled an authenticated terminal; no renewal or Submit replay"
            );
            Ok(())
        }
        _ => Err(Error::Conflict),
    }
}
async fn send_open(service: &Service, actor: &Actor, command: RemoteOpen) -> Result<Reply> {
    match command {
        RemoteOpen::Legacy(command) => send(service, actor, Request::OpenSession { command }).await,
        RemoteOpen::Broker(command) => {
            service.context.running().map_err(|_| Error::Unauthorized)?;
            actor.running().map_err(|_| Error::Unauthorized)?;
            let reply = actor
                .transport
                .open_broker_session(*command.clone())
                .await
                .map_err(|_| Error::Uncertain)?;
            let Reply::SessionAccepted { receipt } = &reply else {
                return Err(Error::Conflict);
            };
            command
                .validate_receipt(receipt)
                .map_err(|_| Error::Conflict)?;
            Ok(reply)
        }
    }
}
async fn submit(
    service: &Arc<Service>,
    actor: &Arc<Actor>,
    message: Request,
    deadline: Instant,
) -> Result<()> {
    let Request::SubmitTurn { command } = &message else {
        return Err(Error::Conflict);
    };
    let deadline = deadline.min(
        actor
            .turn
            .lock()
            .map_err(|_| Error::Unavailable)?
            .submit_deadline(command, Instant::now())
            .map_err(|_| Error::Unauthorized)?,
    );
    let identity = command.identity.clone();
    match tokio::time::timeout_at(deadline, send(service, actor, message)).await {
        Ok(Ok(reply)) => events::persist(service, actor.transport.clone(), reply).await,
        Ok(Err(Error::Uncertain)) if Instant::now() < deadline => {
            actor.running().map_err(|_| Error::Unauthorized)?;
            let owner = service.clone();
            let current = actor.clone();
            let receipt = service
                .context
                .authority(move |ctx, store, policy| {
                    ctx.running()?;
                    current.running()?;
                    store.remote_binding_transaction(false, |store| {
                        let scope = mcp::scope(&current)?;
                        let runtime = scope
                            .as_ref()
                            .map(|scope| scope.runtime(store, &owner.config))
                            .unwrap_or_else(|| store.remote_session_runtime());
                        runtime.resolve_remote_turn_receipt_in_transaction(
                            &current.id,
                            &identity,
                            |store, snapshot| {
                                ctx.running()?;
                                current.authorize(snapshot)?;
                                owner.source(store, snapshot, &current.transport, false)?;
                                ctx.caller(store, &snapshot.owner, &snapshot.device, policy)
                            },
                        )
                    })
                })
                .await?;
            if receipt.is_some() {
                Ok(())
            } else {
                Err(Error::Uncertain)
            }
        }
        Ok(Err(error)) => Err(error),
        Err(_) => Err(Error::Uncertain),
    }
}
async fn control(service: Weak<Service>, actor: Arc<Actor>) -> Result<()> {
    let owner = service.upgrade().ok_or(Error::Unavailable)?;
    let Output::Open(command) = positive(&owner, &actor, Cut::Open).await? else {
        return Err(Error::Conflict);
    };
    let ttl = Duration::from_millis(command.budgets.lease_duration_ms);
    let mut lease_deadline = *actor
        .lease_deadline
        .lock()
        .map_err(|_| Error::Unavailable)?;
    let reply = tokio::time::timeout_at(lease_deadline, send_open(&owner, &actor, command))
        .await
        .map_err(|_| Error::Uncertain)??;
    events::persist(&owner, actor.transport.clone(), reply).await?;
    drop(owner);
    let mut renew_at = lease_deadline - ttl.mul_f64(0.6);
    loop {
        actor.running().map_err(|_| Error::Unauthorized)?;
        let owner = service.upgrade().ok_or(Error::Unavailable)?;
        let id = actor.id.clone();
        let (snapshot, activity) = owner
            .context
            .sql(move |_, store| {
                Ok((
                    store.remote_session_snapshot(&id)?,
                    store.remote_session_activity(&id)?,
                ))
            })
            .await?;
        actor
            .authorize(&snapshot)
            .map_err(|_| Error::Unauthorized)?;
        actor
            .turn
            .lock()
            .map_err(|_| Error::Unavailable)?
            .reconcile(activity.active_turn.as_deref())
            .map_err(|_| Error::Unauthorized)?;
        if snapshot.phase == "stopped" {
            return Ok(());
        }
        if snapshot.closing || snapshot.phase == "interrupted" || activity.cancellation {
            return Err(Error::Uncertain);
        }
        if Instant::now() >= renew_at {
            if Instant::now() >= lease_deadline {
                return Err(Error::Uncertain);
            }
            let operation = async {
                let Output::Request(message) = positive(&owner, &actor, Cut::Renew).await? else {
                    return Err(Error::Conflict);
                };
                let sent = Instant::now();
                send(&owner, &actor, message).await?;
                if let Some(turn) = activity.active_turn.clone()
                    && let Output::Request(message) =
                        positive(&owner, &actor, Cut::RenewTurn(turn)).await?
                {
                    renew_turn(&owner, &actor, message).await?;
                }
                Ok(sent + ttl)
            };
            lease_deadline = tokio::time::timeout_at(lease_deadline, operation)
                .await
                .map_err(|_| Error::Uncertain)??;
            actor.running().map_err(|_| Error::Uncertain)?;
            *actor
                .lease_deadline
                .lock()
                .map_err(|_| Error::Unavailable)? = lease_deadline;
            renew_at = lease_deadline - ttl.mul_f64(0.6);
        }
        if snapshot.phase == "active"
            && activity.active_turn.is_none()
            && activity.queued
            && let Output::Claim(Some(command)) = positive(&owner, &actor, Cut::Claim).await?
        {
            actor
                .turn
                .lock()
                .map_err(|_| Error::Unavailable)?
                .begin(command.clone(), actor.deadline, Instant::now())
                .map_err(|_| Error::Unauthorized)?;
            let context = owner.context.clone();
            context
                .sql(move |_, store| {
                    store
                        .interactive_journal()
                        .notify_committed_claim()
                        .map_err(|_| denied())
                })
                .await?;
            let Output::Request(message) =
                positive(&owner, &actor, Cut::Submit(command.identity.turn_id)).await?
            else {
                return Err(Error::Conflict);
            };
            submit(&owner, &actor, message, lease_deadline).await?;
        }
        drop(owner);
        tokio::select! {_=tokio::time::sleep(Duration::from_millis(100))=>{},_=actor.changed.notified()=>{}}
    }
}
async fn monitor(service: Weak<Service>, actor: Arc<Actor>) -> Result<()> {
    loop {
        actor.running().map_err(|_| Error::Unauthorized)?;
        let owner = service.upgrade().ok_or(Error::Unavailable)?;
        let service = owner.clone();
        let current = actor.clone();
        let result = owner
            .context
            .authority(move |ctx, store, policy| {
                ctx.running()?;
                current.running()?;
                store.remote_binding_transaction(false, |store| {
                    let scope = mcp::scope(&current)?;
                    let runtime = scope
                        .as_ref()
                        .map(|scope| scope.runtime(store, &service.config))
                        .unwrap_or_else(|| store.remote_session_runtime());
                    runtime.validate_remote_session_actor_in_transaction(
                        &current.id,
                        |store, snapshot| {
                            ctx.running()?;
                            current.authorize(snapshot)?;
                            service.source(store, snapshot, &current.transport, false)?;
                            ctx.caller(store, &snapshot.owner, &snapshot.device, policy)
                        },
                    )
                })
            })
            .await;
        if let Err(error) = result {
            actor.stop.store(true, Ordering::Release);
            actor.changed.notify_waiters();
            return Err(error);
        }
        drop(owner);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
pub(super) async fn run(service: Weak<Service>, actor: Arc<Actor>, setup: Option<AppState>) {
    let outcome = async {
        if let Some(state) = setup {
            let owner = service.upgrade().ok_or(Error::Unavailable)?;
            let deadline = *actor.lease_deadline.lock().map_err(|_| Error::Unavailable)?;
            tokio::time::timeout_at(deadline, mcp::install(&state, &owner, &actor)).await
                .map_err(|_| Error::Uncertain)?
                .map_err(|_| Error::Unauthorized)?;
        }
        let exchange = control(service.clone(), actor.clone());
        let guard = monitor(service.clone(), actor.clone());
        tokio::pin!(exchange);
        tokio::pin!(guard);
        loop {
            tokio::select! {
                result=&mut exchange=>break result,
                result=&mut guard=>break result,
                _=tokio::time::sleep_until(actor.deadline)=>break Err(Error::Uncertain),
                _=actor.changed.notified()=>{if actor.finished.load(Ordering::Acquire){break Ok(());}else if actor.stop.load(Ordering::Acquire){break Err(Error::Uncertain);}}
            }
        }
    }.await;
    #[cfg(test)]
    if let Err(reason) = outcome {
        eprintln!("controller session actor stopped: {reason:?}");
    }
    #[cfg(not(test))]
    let _ = outcome;
    actor.stop.store(true, Ordering::Release);
    if let Some(owner) = service.upgrade() {
        // Negative cancellation must not wait for filesystem policy or caller
        // authorization. The original receipt is the sole stop capability.
        let mut receipt = actor
            .receipt
            .lock()
            .ok()
            .and_then(|receipt| receipt.clone());
        if receipt.is_none() {
            receipt = owner
                .snapshot(&actor.id)
                .await
                .ok()
                .and_then(|snapshot| snapshot.receipt);
        }
        if let Some(receipt) = receipt {
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                actor.transport.request(Request::CloseSession {
                    session: receipt.fence,
                }),
            )
            .await;
        }
        let id = actor.id.clone();
        let scope = owner.clone();
        let _ = owner
            .context
            .sql(move |_, store| {
                store.remote_binding_transaction(false, |store| {
                    store.interrupt_remote_session_in_transaction(&id, |_, snapshot| {
                        scope.scope(snapshot)
                    })
                })?;
                store
                    .interactive_journal()
                    .notify_committed_update()
                    .map_err(|_| denied())
            })
            .await;
        // Cleanup retains the durable quarantine until an actual worker stop.
        // Dropping this actor never grants another worker admission.
        owner.finished(&actor.id);
    }
}
