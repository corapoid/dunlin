use super::*;
use dunlin_relay_protocol::worker::session::{
    SessionFence,
    broker::{BrokerReplyChunk, chunk_reply},
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

fn pair(pending: u8, bytes: u32) -> (Client, SocketOwner, Arc<AtomicBool>) {
    let live = Arc::new(AtomicBool::new(true));
    let guard = live.clone();
    let (client, owner) = SocketOwner::new(
        ChannelFence {
            channel_id: "channel".into(),
            controller_boot_id: "controller".into(),
            worker_boot_id: "worker".into(),
        },
        BrokerLimits {
            max_pending: pending,
            max_reply_bytes: bytes,
        },
        Arc::new(move || {
            if guard.load(Ordering::Acquire) {
                Ok(())
            } else {
                Err(Error::Closed)
            }
        }),
    )
    .unwrap();
    (client, owner, live)
}
fn scope(client: &Client) -> BrokerScope {
    BrokerScope::Catalog {
        session: SessionFence {
            channel: client.fence.clone(),
            controller_session_id: "session".into(),
            worker_session_id: "worker-session".into(),
            session_epoch: 1,
        },
    }
}
fn list() -> BrokerOperation {
    BrokerOperation::ToolsList { cursor: None }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
fn submit(client: &Client) -> Answer {
    client.submit(scope(client), list(), deadline()).unwrap()
}
async fn dispatched(owner: &mut SocketOwner) -> BrokerRequest {
    let BrokerWorkerMessage::McpRequest { request } = owner.next().await.unwrap().message;
    request
}
fn deliver(owner: &mut SocketOwner, chunk: BrokerReplyChunk) -> Result<()> {
    owner.accept(BrokerControllerMessage::McpReplyChunk { chunk })
}

#[tokio::test]
async fn interleaved_responses_are_correlated_without_blocking_other_socket_work() {
    let (client, mut owner, _) = pair(2, 262144);
    let mut first = submit(&client);
    let request = dispatched(&mut owner).await;
    let second = submit(&client);
    let other = dispatched(&mut owner).await;
    assert_ne!(request.request_id, other.request_id);
    let payload = serde_json::to_string(&"ą".repeat(10000)).unwrap();
    let chunks = chunk_reply(&request, &payload).unwrap();
    deliver(&mut owner, chunks[0].clone()).unwrap();
    assert!(matches!(
        first.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    deliver(&mut owner, chunk_reply(&other, "{}").unwrap().remove(0)).unwrap();
    assert_eq!(second.await.unwrap().unwrap(), "{}");
    for chunk in chunks.into_iter().skip(1) {
        deliver(&mut owner, chunk).unwrap();
    }
    assert_eq!(first.await.unwrap().unwrap(), payload);
    assert_eq!(client.slots.available_permits(), 2);
}

#[tokio::test]
async fn canceled_dispatched_waiter_keeps_capacity_until_real_completion() {
    let (client, mut owner, _) = pair(1, 262144);
    let answer = submit(&client);
    let request = dispatched(&mut owner).await;
    drop(answer);
    assert!(matches!(
        client.submit(scope(&client), list(), deadline()),
        Err(Error::Capacity)
    ));
    deliver(&mut owner, chunk_reply(&request, "{}").unwrap().remove(0)).unwrap();
    assert_eq!(client.slots.available_permits(), 1);
    let _answer = submit(&client);
    let next = dispatched(&mut owner).await;
    assert_ne!(request.request_id, next.request_id);
}

#[tokio::test]
async fn canceled_unsent_work_is_skipped_and_foreign_channel_never_enters_queue() {
    let (client, mut owner, _) = pair(2, 262144);
    let canceled = submit(&client);
    drop(canceled);
    let answer = submit(&client);
    let request = dispatched(&mut owner).await;
    assert_eq!(owner.pending.len(), 1);
    assert_eq!(client.slots.available_permits(), 1);
    let mut foreign = scope(&client);
    let BrokerScope::Catalog { session } = &mut foreign else {
        unreachable!()
    };
    session.channel.worker_boot_id = "another-boot".into();
    assert!(matches!(
        client.submit(foreign, list(), deadline()),
        Err(Error::Invalid)
    ));
    owner
        .accept(BrokerControllerMessage::McpRejected {
            request: request.reference().unwrap(),
            code: BrokerError::Denied,
        })
        .unwrap();
    assert_eq!(
        answer.await.unwrap(),
        Err(Error::Rejected(BrokerError::Denied))
    );
}

#[tokio::test]
async fn changed_scope_or_reordered_reply_closes_pair_and_all_dispatched_work() {
    for changed_scope in [false, true] {
        let (client, mut owner, _) = pair(2, 262144);
        let first = submit(&client);
        let request = dispatched(&mut owner).await;
        let second = submit(&client);
        let _other = dispatched(&mut owner).await;
        let payload = serde_json::to_string(&"x".repeat(10000)).unwrap();
        let mut chunk = chunk_reply(&request, &payload).unwrap().remove(1);
        if changed_scope {
            chunk.index = 0;
            let BrokerScope::Catalog { session } = &mut chunk.request.scope else {
                unreachable!()
            };
            session.session_epoch += 1;
        }
        assert_eq!(deliver(&mut owner, chunk), Err(Error::Invalid));
        assert!(first.await.is_err());
        assert!(second.await.is_err());
        assert!(matches!(
            client.submit(scope(&client), list(), deadline()),
            Err(Error::Closed)
        ));
    }
}

#[tokio::test]
async fn negotiated_reply_limit_is_checked_before_allocating_the_assembly() {
    let (client, mut owner, _) = pair(1, 32);
    let answer = submit(&client);
    let request = dispatched(&mut owner).await;
    let payload = serde_json::to_string(&"x".repeat(100)).unwrap();
    assert_eq!(
        deliver(
            &mut owner,
            chunk_reply(&request, &payload).unwrap().remove(0)
        ),
        Err(Error::Invalid)
    );
    assert!(answer.await.is_err());
    assert!(owner.closed);
}

#[tokio::test]
async fn expiry_and_channel_drop_never_requeue_dispatched_operations() {
    let (client, mut owner, _) = pair(1, 262144);
    let until = deadline();
    let answer = client.submit(scope(&client), list(), until).unwrap();
    let request = dispatched(&mut owner).await;
    assert_eq!(owner.expire_at(until), Err(Error::OutcomeUnknown));
    assert!(answer.await.is_err());
    assert!(owner.pending.is_empty());
    assert!(matches!(
        owner.queue.try_recv(),
        Err(mpsc::error::TryRecvError::Disconnected)
    ));
    assert_eq!(client.slots.available_permits(), 1);
    // A late reply cannot be attributed to another operation or a new pair.
    assert_eq!(
        deliver(&mut owner, chunk_reply(&request, "{}").unwrap().remove(0)),
        Err(Error::Closed)
    );
    let (client, mut owner, _) = pair(1, 262144);
    let waiting = client.clone();
    let task =
        tokio::spawn(async move { waiting.request(scope(&waiting), list(), deadline()).await });
    let _request = dispatched(&mut owner).await;
    drop(owner);
    assert_eq!(task.await.unwrap(), Err(Error::OutcomeUnknown));
    assert!(matches!(
        client.submit(scope(&client), list(), deadline()),
        Err(Error::Closed)
    ));
}

#[tokio::test]
async fn fresh_channel_guard_runs_before_dispatch_and_again_before_delivery() {
    let (client, mut owner, live) = pair(1, 262144);
    let answer = submit(&client);
    live.store(false, Ordering::Release);
    assert_eq!(owner.next().await.unwrap_err(), Error::Closed);
    assert_eq!(answer.await.unwrap(), Err(Error::Closed));
    assert!(owner.pending.is_empty());
    let (client, mut owner, live) = pair(1, 262144);
    let answer = submit(&client);
    let request = dispatched(&mut owner).await;
    live.store(false, Ordering::Release);
    assert_eq!(
        deliver(&mut owner, chunk_reply(&request, "{}").unwrap().remove(0)),
        Err(Error::Closed)
    );
    assert!(answer.await.is_err());
}

#[tokio::test]
async fn completed_answer_is_rechecked_in_the_waiting_client_before_use() {
    for revoke in [false, true] {
        let (client, mut owner, live) = pair(1, 262144);
        let task =
            tokio::spawn(async move { client.request(scope(&client), list(), deadline()).await });
        let request = dispatched(&mut owner).await;
        deliver(&mut owner, chunk_reply(&request, "{}").unwrap().remove(0)).unwrap();
        // The current-thread test does not yield between sending and revoking.
        if revoke {
            live.store(false, Ordering::Release);
        }
        assert_eq!(
            task.await.unwrap(),
            if revoke {
                Err(Error::Closed)
            } else {
                Ok("{}".into())
            }
        );
    }
}

#[tokio::test]
async fn independent_timer_fences_pending_and_queued_work_and_wait_is_bounded() {
    let (client, mut owner, live) = pair(2, 262144);
    assert!(matches!(
        client.submit(scope(&client), list(), Instant::now()),
        Err(Error::Expired)
    ));
    assert!(matches!(
        client.submit(
            scope(&client),
            list(),
            Instant::now() + Duration::from_secs(151)
        ),
        Err(Error::Invalid)
    ));
    assert_eq!(client.slots.available_permits(), 2);
    let sent = submit(&client);
    let _request = dispatched(&mut owner).await;
    let unsent = submit(&client);
    live.store(false, Ordering::Release);
    assert_eq!(owner.expire(), Err(Error::Closed));
    assert!(owner.closed);
    assert!(sent.await.is_err());
    assert_eq!(unsent.await.unwrap(), Err(Error::Closed));
    assert!(owner.pending.is_empty());
}

#[tokio::test]
async fn outgoing_keeps_exact_deadline_and_reply_frame_decoder_cannot_consume_other_protocols() {
    let (client, mut owner, _) = pair(1, 65536);
    let deadline = deadline();
    let answer = client.submit(scope(&client), list(), deadline).unwrap();
    let outbound = owner.next().await.unwrap();
    assert_eq!(outbound.deadline, deadline);
    let BrokerWorkerMessage::McpRequest { request } = outbound.message;
    assert!(!owner.accept_frame(r#"{"type":"heartbeat_ack"}"#).unwrap());
    assert_eq!(owner.pending.len(), 1);
    let reply = BrokerControllerMessage::McpReplyChunk {
        chunk: chunk_reply(&request, "{}").unwrap().remove(0),
    };
    assert!(
        owner
            .accept_frame(&serde_json::to_string(&reply).unwrap())
            .unwrap()
    );
    assert_eq!(answer.await.unwrap().unwrap(), "{}");
    assert!(client.attest(&client.fence).is_ok());
    let mut foreign = client.fence.clone();
    foreign.controller_boot_id = "foreign".into();
    assert_eq!(client.attest(&foreign), Err(Error::Invalid));
    drop(owner);
    assert_eq!(client.attest(&client.fence), Err(Error::Closed));
}

#[tokio::test]
async fn oversized_or_unmatched_broker_reply_closes_owner_without_releasing_a_partial_answer() {
    for oversized in [false, true] {
        let (client, mut owner, _) = pair(1, 65536);
        let answer = submit(&client);
        let mut request = dispatched(&mut owner).await;
        request.request_id = "unmatched".into();
        let text = if oversized {
            " ".repeat(dunlin_relay_protocol::worker::MAX_WORKER_FRAME_BYTES + 1)
        } else {
            serde_json::to_string(&BrokerControllerMessage::McpRejected {
                request: request.reference().unwrap(),
                code: BrokerError::Denied,
            })
            .unwrap()
        };
        assert_eq!(owner.accept_frame(&text), Err(Error::Invalid));
        assert!(answer.await.is_err());
        assert_eq!(client.attest(&client.fence), Err(Error::Closed));
    }
}

#[tokio::test]
async fn per_request_reply_quota_is_checked_before_assembly_even_with_a_larger_channel_offer() {
    let (client, mut owner, _) = pair(2, 262144);
    assert!(matches!(
        client.submit_bounded(scope(&client), list(), deadline(), 262145),
        Err(Error::Invalid)
    ));
    let answer = client
        .submit_bounded(scope(&client), list(), deadline(), 1024)
        .unwrap();
    let request = dispatched(&mut owner).await;
    let payload = serde_json::to_string(&"x".repeat(2048)).unwrap();
    assert_eq!(
        deliver(
            &mut owner,
            chunk_reply(&request, &payload).unwrap().remove(0)
        ),
        Err(Error::Invalid)
    );
    assert!(answer.await.is_err());
    assert!(owner.closed);
}
