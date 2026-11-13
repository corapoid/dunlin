use super::*;

pub(super) async fn persist(
    service: &Arc<Service>,
    transport: SessionTransport,
    message: Reply,
) -> Result<()> {
    let id = match &message {
        Reply::SessionAccepted { receipt } | Reply::SessionReady { receipt } => {
            receipt.fence.controller_session_id.clone()
        }
        Reply::TurnAccepted { receipt } => receipt.fence.session.controller_session_id.clone(),
        Reply::Event { event } => event.session.controller_session_id.clone(),
        _ => return Err(Error::Conflict),
    };
    #[cfg(test)]
    let host = transport.host_id().to_owned();
    let ready = matches!(&message, Reply::SessionReady { .. });
    let receipt = match &message {
        Reply::SessionAccepted { receipt } | Reply::SessionReady { receipt } => {
            Some(receipt.clone())
        }
        _ => None,
    };
    let owner = service.clone();
    let session = id.clone();
    let cut = service
        .context
        .authority(move |ctx, store, policy| {
            let current = owner
                .actors
                .lock()
                .ok()
                .and_then(|actors| actors.get(&session).cloned());
            // Missing/poisoned live proof can only select legacy rejection or
            // negative receipt/stop cleanup, never positive MCP projection.
            let scope = current
                .as_ref()
                .and_then(|actor| mcp::scope(actor).ok().flatten());
            let cut = store.remote_binding_transaction(false, |store| {
                let runtime = scope
                    .as_ref()
                    .map(|scope| scope.runtime(store, &owner.config))
                    .unwrap_or_else(|| store.remote_session_runtime());
                match message {
                    Reply::SessionAccepted { receipt } | Reply::SessionReady { receipt } => {
                        let snapshot = store.remote_session_snapshot(&session)?;
                        let stopping = owner
                            .actors
                            .lock()
                            .ok()
                            .and_then(|actors| actors.get(&session).cloned())
                            .is_none_or(|actor| actor.running().is_err());
                        if stopping
                            || snapshot.closing
                            || snapshot.phase == "interrupted"
                            || ctx.is_stopped()
                        {
                            store.capture_remote_session_receipt_in_transaction(
                                &session,
                                &receipt,
                                |store, snapshot| owner.source(store, snapshot, &transport, true),
                            )?;
                        } else {
                            runtime.accept_remote_session_in_transaction(
                                &session,
                                &receipt,
                                ready,
                                |store, snapshot| {
                                    ctx.running()?;
                                    if scope.is_some() {
                                        current.as_ref().ok_or_else(denied)?.authorize(snapshot)?;
                                    }
                                    owner.source(store, snapshot, &transport, false)?;
                                    ctx.caller(store, &snapshot.owner, &snapshot.device, policy)
                                },
                            )?;
                        }
                        Ok(None)
                    }
                    Reply::TurnAccepted { receipt } => {
                        runtime.accept_remote_session_turn_in_transaction(
                            &session,
                            &receipt,
                            |store, snapshot| {
                                ctx.running()?;
                                if scope.is_some() {
                                    current.as_ref().ok_or_else(denied)?.authorize(snapshot)?;
                                }
                                owner.source(store, snapshot, &transport, false)?;
                                ctx.caller(store, &snapshot.owner, &snapshot.device, policy)
                            },
                        )?;
                        Ok(None)
                    }
                    Reply::Event { event } => runtime
                        .record_remote_session_event_in_transaction(
                            &session,
                            &event,
                            |store, snapshot| owner.source(store, snapshot, &transport, true),
                            |store, user, device| {
                                if scope.is_some() {
                                    ctx.running()?;
                                    current.as_ref().ok_or_else(denied)?.running()?;
                                }
                                ctx.caller(store, user, device, policy)
                            },
                        )
                        .map(Some),
                    _ => Err(denied()),
                }
            })?;
            // A notification is emitted only after the entire source inbox and
            // Journal transaction commits. It carries no execution authority.
            store
                .interactive_journal()
                .notify_committed_update()
                .map_err(|_| denied())?;
            Ok(cut)
        })
        .await?;
    #[cfg(test)]
    if matches!(&cut, Some(RemoteStreamCut::Gap { .. })) {
        crate::worker_client::session_faults::gap(&host, &id);
    }
    if let Ok(actors) = service.actors.lock()
        && let Some(actor) = actors.get(&id)
    {
        if let Some(receipt) = receipt
            && let Ok(mut cached) = actor.receipt.lock()
        {
            *cached = Some(receipt);
        }
        actor.changed.notify_waiters();
    }
    match cut {
        Some(RemoteStreamCut::Gap { .. }) => {} // bounded cleanup replays from the durable anchor
        Some(RemoteStreamCut::Persisted { stopped: true, .. }) => service.finished(&id),
        _ => {}
    }
    Ok(())
}
pub(super) async fn consume(service: Weak<Service>, mut receiver: mpsc::Receiver<SessionEvent>) {
    while let Some(event) = receiver.recv().await {
        let Some(owner) = service.upgrade() else {
            return;
        };
        let id = match &event.message {
            Reply::SessionAccepted { receipt } | Reply::SessionReady { receipt } => {
                Some(receipt.fence.controller_session_id.clone())
            }
            Reply::TurnAccepted { receipt } => {
                Some(receipt.fence.session.controller_session_id.clone())
            }
            Reply::Event { event } => Some(event.session.controller_session_id.clone()),
            _ => None,
        };
        let fence = event.transport.fence().clone();
        if persist(&owner, event.transport, event.message)
            .await
            .is_err()
            && let Some(id) = id
            && let Ok(actors) = owner.actors.lock()
            && let Some(actor) = actors.get(&id)
            && actor.transport.fence() == &fence
        {
            actor.stop.store(true, Ordering::Release);
            actor.changed.notify_waiters();
        }
    }
}
async fn acknowledge(
    service: &Arc<Service>,
    snapshot: &RemoteSessionSnapshot,
    transport: &SessionTransport,
) -> Result<()> {
    if snapshot.ack_sequence <= snapshot.confirmed_ack {
        return Ok(());
    }
    let request = Request::AckStream {
        channel: transport.fence().clone(),
        session: snapshot
            .receipt
            .as_ref()
            .ok_or(Error::Conflict)?
            .fence
            .clone(),
        through_sequence: snapshot.ack_sequence,
        event_digest: snapshot.ack_digest.clone().ok_or(Error::Conflict)?,
    };
    #[cfg(test)]
    crate::worker_client::session_faults::pause_ack(
        transport.host_id(),
        crate::worker_client::session_faults::AckCut::ControllerBeforeWire,
        snapshot.ack_sequence,
    )
    .await;
    let reply = transport
        .request(request.clone())
        .await
        .map_err(|_| Error::Unavailable)?;
    reply
        .validate_for_request(&request)
        .map_err(|_| Error::Conflict)?;
    if !matches!(reply, Reply::StreamAcknowledged { .. }) {
        return Err(Error::Conflict);
    }
    let id = snapshot.session.clone();
    let source = transport.clone();
    let owner = service.clone();
    service
        .context
        .sql(move |_, store| {
            store.remote_binding_transaction(false, |store| {
                store.confirm_remote_session_ack_in_transaction(
                    &id,
                    &request,
                    |store, snapshot, request| {
                        owner.source(store, snapshot, &source, true)?;
                        source.attest_stream_ack_in_transaction(store, request)
                    },
                )
            })
        })
        .await
}
pub(super) async fn replay(
    service: &Arc<Service>,
    snapshot: &RemoteSessionSnapshot,
    transport: &SessionTransport,
) -> Result<()> {
    let Some(receipt) = &snapshot.receipt else {
        return Ok(());
    };
    let request = Request::ReplayStream {
        channel: transport.fence().clone(),
        session: receipt.fence.clone(),
        after_sequence: snapshot.sequence,
        after_digest: snapshot.digest.clone(),
        limit: 8,
    };
    let reply = transport
        .request(request.clone())
        .await
        .map_err(|_| Error::Unavailable)?;
    reply
        .validate_for_request(&request)
        .map_err(|_| Error::Conflict)?;
    if let Reply::StreamReplayed { events, .. } = reply {
        for event in events {
            persist(service, transport.clone(), Reply::Event { event }).await?;
        }
    }
    Ok(())
}
async fn reconcile(
    service: &Arc<Service>,
    snapshot: &RemoteSessionSnapshot,
    transport: &SessionTransport,
) -> Result<()> {
    let request = Request::ReconcileSession {
        channel: transport.fence().clone(),
        identity: snapshot.open.identity.clone(),
    };
    let reply = transport
        .request(request.clone())
        .await
        .map_err(|_| Error::Unavailable)?;
    reply
        .validate_for_request(&request)
        .map_err(|_| Error::Conflict)?;
    let Reply::SessionReconciled {
        record: Some(record),
        ..
    } = reply
    else {
        return Ok(());
    };
    let id = snapshot.session.clone();
    let source = transport.clone();
    let owner = service.clone();
    if record.phase == wire::SessionPhase::Stopped {
        service
            .context
            .sql(move |_, store| {
                store.remote_binding_transaction(false, |store| {
                    store.reconcile_remote_session_stop_in_transaction(
                        &id,
                        &record,
                        |store, snapshot, _| owner.source(store, snapshot, &source, true),
                    )
                })?;
                store
                    .interactive_journal()
                    .notify_committed_update()
                    .map_err(|_| denied())
            })
            .await?;
        service.finished(&snapshot.session);
    } else {
        let receipt = record.receipt;
        let close = service
            .context
            .sql(move |_, store| {
                store.remote_binding_transaction(false, |store| {
                    store.capture_remote_session_receipt_in_transaction(
                        &id,
                        &receipt,
                        |store, snapshot| owner.source(store, snapshot, &source, true),
                    )?;
                    store.close_remote_session_in_transaction(&id, |store, snapshot| {
                        owner.source(store, snapshot, &source, true)
                    })
                })
            })
            .await?;
        if let Some(request) = close {
            let _ = transport.request(request).await;
        }
    }
    Ok(())
}
pub(super) async fn cleanup(service: Weak<Service>) {
    let mut after = String::new();
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let Some(owner) = service.upgrade() else {
            return;
        };
        let Ok(ids) = owner
            .context
            .sql(|_, store| store.remote_session_cleanup_ids())
            .await
        else {
            continue;
        };
        let page: Vec<_> = ids
            .iter()
            .filter(|id| id.as_str() > after.as_str())
            .take(8)
            .cloned()
            .collect();
        if page.is_empty() {
            after.clear();
            continue;
        }
        for id in page {
            after = id.clone();
            let Ok(snapshot) = owner.snapshot(&id).await else {
                continue;
            };
            let Ok(transport) = owner
                .channels
                .session_transport(&snapshot.open.descriptor.scope.host_id)
            else {
                continue;
            };
            let _ = acknowledge(&owner, &snapshot, &transport).await;
            let _ = replay(&owner, &snapshot, &transport).await;
            let Ok(snapshot) = owner.snapshot(&id).await else {
                continue;
            };
            if snapshot.closing || snapshot.phase == "interrupted" {
                let _ = reconcile(&owner, &snapshot, &transport).await;
            }
        }
    }
}
