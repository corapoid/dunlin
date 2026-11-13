//! Real verified worker socket and native runtime, with a transport-only host
//! responder. These probes do not create a private MCP account or native tool grant.
use super::*;
use crate::{
    worker_channel::{SessionBroker, SessionTransport},
    worker_client::broker::{self, fixture::get},
};
use dunlin_relay_protocol::worker::session::{SessionReceipt, broker::*};
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::{sync::Semaphore, time::Instant};

struct Responder {
    session: String,
    gate: Semaphore,
    calls: AtomicUsize,
    stopped: AtomicBool,
    payload: String,
}
impl SessionBroker for Responder {
    fn session(&self) -> &str {
        &self.session
    }
    fn is_live(&self) -> bool {
        !self.stopped.load(Ordering::Acquire)
    }
    fn interrupt(&self) {
        self.stopped.store(true, Ordering::Release);
    }
    fn request(
        self: Arc<Self>,
        request: BrokerRequest,
        _: Instant,
    ) -> Pin<Box<dyn Future<Output = Result<String, BrokerError>> + Send>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::Release);
            if matches!(
                request.operation,
                BrokerOperation::ToolsList { cursor: Some(_) }
            ) {
                return Ok("{\"tools\":[]}".into());
            }
            self.gate.acquire().await.unwrap().forget();
            Ok(self.payload.clone())
        })
    }
}
fn responder(receipt: &SessionReceipt) -> Arc<Responder> {
    Arc::new(Responder {
        session: receipt.fence.controller_session_id.clone(),
        gate: Semaphore::new(0),
        calls: AtomicUsize::new(0),
        stopped: AtomicBool::new(false),
        payload: json!({"tools":[],"fixture":"ą".repeat(40000)}).to_string(),
    })
}
fn heartbeat(f: &Fixture) -> u64 {
    f.state
        .store
        .lock()
        .unwrap()
        .host_health_observation(&f.owner, &f.workspace, &f.host, &f.state.worker_channels)
        .unwrap()
        .connection
        .unwrap()
        .heartbeat_sequence
}
async fn began(handler: &Responder) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while handler.calls.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
pub(super) async fn interleaved(
    f: &Fixture,
    transport: &SessionTransport,
    receipt: &SessionReceipt,
) {
    let client =
        get(&f.host, transport.fence()).expect("client published only by verified v5 source loop");
    let handler = responder(receipt);
    let _binding = transport.bind_broker(handler.clone()).unwrap();
    let baseline = heartbeat(f);
    let scope = BrokerScope::Catalog {
        session: receipt.fence.clone(),
    };
    let caller = client.clone();
    let first_scope = scope.clone();
    let first = tokio::spawn(async move {
        caller
            .request(
                first_scope,
                BrokerOperation::ToolsList { cursor: None },
                Instant::now() + Duration::from_secs(10),
            )
            .await
    });
    began(&handler).await;
    let other = client
        .request(
            scope,
            BrokerOperation::ToolsList {
                cursor: Some("controlled-fast-page".into()),
            },
            Instant::now() + Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert_eq!(other, "{\"tools\":[]}");
    assert!(!first.is_finished());
    tokio::time::timeout(Duration::from_secs(5), async {
        while heartbeat(f) <= baseline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("heartbeat must advance while MCP response is held");
    assert!(!first.is_finished());
    handler.gate.add_permits(1);
    let answer = first.await.unwrap().unwrap();
    assert_eq!(answer, handler.payload);
    assert_eq!(handler.calls.load(Ordering::Acquire), 2);
    assert!(client.attest(transport.fence()).is_ok());
}

pub(super) async fn lost_reply(
    f: &Fixture,
    transport: &SessionTransport,
    receipt: &SessionReceipt,
) {
    let client = get(&f.host, transport.fence()).expect("verified source client required");
    let handler = responder(receipt);
    let _binding = transport.bind_broker(handler.clone()).unwrap();
    let caller = client.clone();
    let scope = BrokerScope::Catalog {
        session: receipt.fence.clone(),
    };
    let request = tokio::spawn(async move {
        caller
            .request(
                scope,
                BrokerOperation::ToolsList { cursor: None },
                Instant::now() + Duration::from_secs(1),
            )
            .await
    });
    began(&handler).await;
    assert_eq!(request.await.unwrap(), Err(broker::Error::OutcomeUnknown));
    tokio::time::timeout(Duration::from_secs(3), async {
        while client.attest(transport.fence()).is_ok() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("independent timer must fence source channel after uncertain reply");
    assert_eq!(handler.calls.load(Ordering::Acquire), 1);
}

// Source proxy acceptance uses a controlled tools responder. Controller OAuth
// and SDK consent are a separate integration layer, never manufactured here.
pub(super) struct Native {
    session: String,
    gate: Semaphore,
    calls: AtomicUsize,
    stopped: AtomicBool,
    text: String,
}
impl Native {
    pub(super) async fn wait_call(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.calls.load(Ordering::Acquire) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    pub(super) fn release(&self) {
        self.gate.add_permits(1);
    }
    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }
}
impl SessionBroker for Native {
    fn session(&self) -> &str {
        &self.session
    }
    fn is_live(&self) -> bool {
        !self.stopped.load(Ordering::Acquire)
    }
    fn interrupt(&self) {
        self.stopped.store(true, Ordering::Release);
    }
    fn request(
        self: Arc<Self>,
        request: BrokerRequest,
        _: Instant,
    ) -> Pin<Box<dyn Future<Output = Result<String, BrokerError>> + Send>> {
        Box::pin(async move {
            let alias = format!("mcp_{}", "a".repeat(32));
            let result = match request.operation {
                BrokerOperation::ToolsList { cursor } => {
                    assert!(cursor.is_none());
                    assert!(matches!(request.scope, BrokerScope::Catalog { .. }));
                    json!({"tools":[{"name":alias,"inputSchema":{"type":"object"}}]})
                }
                BrokerOperation::ToolsCall {
                    name,
                    arguments_json,
                } => {
                    assert_eq!(name, alias);
                    assert_eq!(
                        serde_json::from_str::<Value>(&arguments_json).unwrap(),
                        json!({"query":"native fixture"})
                    );
                    assert!(matches!(request.scope, BrokerScope::Turn { .. }));
                    self.calls.fetch_add(1, Ordering::Release);
                    self.gate.acquire().await.unwrap().forget();
                    json!({"content":[{"type":"text","text":self.text}]})
                }
            };
            Ok(result.to_string())
        })
    }
}
pub(super) fn native(
    transport: &SessionTransport,
    session: &str,
    mode: &str,
) -> (crate::worker_channel::BrokerRegistration, Arc<Native>) {
    let handler = Arc::new(Native {
        session: session.into(),
        gate: Semaphore::new(if mode == "revoke" { 0 } else { 128 }),
        calls: AtomicUsize::new(0),
        stopped: AtomicBool::new(false),
        text: if mode == "quota" {
            "x".repeat(20000)
        } else {
            "native broker result".into()
        },
    });
    (transport.bind_broker(handler.clone()).unwrap(), handler)
}
