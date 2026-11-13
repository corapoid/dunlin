//! Test-only packet loss/reordering after actual worker SQL operations. There is
//! no production config switch, fabricated receipt, bypassed guard or alternate ACP.
use dunlin_relay_protocol::worker::session::{
    SessionControllerMessage as Request, SessionWorkerMessage as Reply,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
static PLAN: Mutex<Option<Arc<Plan>>> = Mutex::new(None);
struct Plan {
    host: String,
    session: Mutex<Option<String>>,
    deferred: Mutex<Vec<Reply>>,
    gap: AtomicBool,
    text_dropped: AtomicBool,
    turn_ack_dropped: AtomicBool,
    ack_dropped: AtomicBool,
    replayed_text: AtomicBool,
    opens: Mutex<usize>,
    submits: Mutex<BTreeMap<String, usize>>,
    renew_release: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    terminal_seen: AtomicBool,
    renew_race: AtomicBool,
}
pub(crate) struct Guard(Arc<Plan>);
impl Guard {
    pub(crate) fn install(host: &str) -> Self {
        let plan = Arc::new(Plan {
            host: host.into(),
            session: Mutex::new(None),
            deferred: Mutex::new(Vec::new()),
            gap: AtomicBool::new(false),
            text_dropped: AtomicBool::new(false),
            turn_ack_dropped: AtomicBool::new(false),
            ack_dropped: AtomicBool::new(false),
            replayed_text: AtomicBool::new(false),
            opens: Mutex::new(0),
            submits: Mutex::new(BTreeMap::new()),
            renew_release: Mutex::new(None),
            terminal_seen: AtomicBool::new(false),
            renew_race: AtomicBool::new(false),
        });
        let mut current = PLAN.lock().unwrap();
        assert!(current.is_none(), "session fault fixture already installed");
        *current = Some(plan.clone());
        Self(plan)
    }
    pub(crate) fn arm_renew_terminal_race(&self, release: Arc<dyn Fn() + Send + Sync>) {
        *self.0.renew_release.lock().unwrap() = Some(release);
    }
    pub(crate) fn assert_renew_terminal_race(&self) {
        assert!(
            self.0.renew_race.load(Ordering::Acquire),
            "renew/terminal race was not exercised"
        );
    }
    pub(crate) fn assert_single_turn_no_replay(&self) {
        assert!(self.0.turn_ack_dropped.load(Ordering::Acquire));
        assert_eq!(*self.0.opens.lock().unwrap(), 1, "Open was resent");
        let submits = self.0.submits.lock().unwrap();
        assert_eq!(submits.len(), 1);
        assert!(
            submits.values().all(|count| *count == 1),
            "Submit was resent"
        );
    }
    pub(crate) fn assert_completed(&self) {
        for (flag, name) in [
            (&self.0.gap, "actual controller gap"),
            (&self.0.text_dropped, "text packet loss"),
            (&self.0.turn_ack_dropped, "turn RPC reply loss"),
            (&self.0.ack_dropped, "durable worker ACK reply loss"),
            (
                &self.0.replayed_text,
                "missing text delivered by ReplayStream",
            ),
        ] {
            assert!(
                flag.load(Ordering::Acquire),
                "fault scenario was not exercised: {name}"
            );
        }
        assert_eq!(*self.0.opens.lock().unwrap(), 1, "Open was resent");
        let submits = self.0.submits.lock().unwrap();
        assert_eq!(submits.len(), 2);
        assert!(
            submits.values().all(|count| *count == 1),
            "Submit was resent after a lost reply"
        );
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if let Ok(mut current) = PLAN.lock()
            && current
                .as_ref()
                .is_some_and(|plan| Arc::ptr_eq(plan, &self.0))
        {
            current.take();
        }
    }
}
fn plan(host: &str, session: Option<&str>) -> Option<Arc<Plan>> {
    let plan = PLAN.lock().ok()?.clone()?;
    if plan.host != host {
        return None;
    }
    if let Some(session) = session {
        let mut pinned = plan.session.lock().ok()?;
        if let Some(pinned) = pinned.as_deref() {
            if pinned != session {
                return None;
            }
        } else {
            *pinned = Some(session.into());
        }
    }
    Some(plan)
}
fn session(reply: &Reply) -> Option<&str> {
    match reply {
        Reply::SessionAccepted { receipt } | Reply::SessionReady { receipt } => {
            Some(&receipt.fence.controller_session_id)
        }
        Reply::TurnAccepted { receipt } => Some(&receipt.fence.session.controller_session_id),
        Reply::Event { event } => Some(&event.session.controller_session_id),
        Reply::StreamReplayed { session, .. } | Reply::StreamAcknowledged { session, .. } => {
            Some(&session.controller_session_id)
        }
        _ => None,
    }
}
pub(crate) async fn before_renew(host: &str, request: &Request) {
    let Request::RenewTurn { turn, .. } = request else {
        return;
    };
    let Some(plan) = plan(host, Some(&turn.session.controller_session_id)) else {
        return;
    };
    let release = { plan.renew_release.lock().unwrap().take() };
    if let Some(release) = release {
        release(); // Release a real accepted HTTP response; never synthesize an event.
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !plan.terminal_seen.load(Ordering::Acquire) {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("actual source terminal before renew deadline");
        plan.renew_race.store(true, Ordering::Release);
    }
}
pub(crate) fn incoming(host: &str, request: &Request) {
    match request {
        Request::OpenSession { command } => {
            if let Some(plan) = plan(host, Some(&command.controller_session_id)) {
                *plan.opens.lock().unwrap() += 1;
            }
        }
        Request::SubmitTurn { command } => {
            if let Some(plan) = plan(host, Some(&command.session.controller_session_id)) {
                *plan
                    .submits
                    .lock()
                    .unwrap()
                    .entry(command.identity.command_id.clone())
                    .or_default() += 1;
            }
        }
        _ => {}
    }
}
pub(crate) fn incoming_broker(
    host: &str,
    command: &dunlin_relay_protocol::worker::session::broker::OpenBrokerSessionCommand,
) {
    if let Some(plan) = plan(host, Some(&command.open.controller_session_id)) {
        *plan.opens.lock().unwrap() += 1;
    }
}
pub(crate) fn gap(host: &str, id: &str) {
    if let Some(plan) = plan(host, Some(id)) {
        plan.gap.store(true, Ordering::Release);
    }
}
pub(crate) fn outgoing(host: &str, replies: Vec<Reply>) -> Vec<Reply> {
    let mut output = Vec::new();
    if let Some(plan) = plan(host, None)
        && plan.gap.load(Ordering::Acquire)
    {
        output.append(&mut plan.deferred.lock().unwrap());
    }
    for reply in replies {
        let Some(plan) = plan(host, session(&reply)) else {
            output.push(reply);
            continue;
        };
        if matches!(&reply,Reply::Event{event} if matches!(event.event,dunlin_relay_protocol::worker::session::SessionEventKind::TurnCompleted{..}))
        {
            plan.terminal_seen.store(true, Ordering::Release);
        }
        if !plan.text_dropped.load(Ordering::Acquire)
            && let Reply::StreamReplayed { events, .. } = &reply
            && let Some(event) = events.iter().find(|event| event.sequence == 1)
        {
            output.push(Reply::Event {
                event: event.clone(),
            });
        }
        if !plan.gap.load(Ordering::Acquire)
            && let Reply::StreamReplayed { events, .. } = &reply
        {
            let terminal = events.iter().find(|event| event.sequence == 2).cloned();
            let mut pending = plan.deferred.lock().unwrap();
            assert!(pending.len() < 8);
            pending.push(reply);
            // Reorder only a canonical, already persisted terminal event. The
            // subsequent real replay reply waits for the controller's Gap cut.
            if let Some(event) = terminal {
                output.push(Reply::Event { event });
                output.append(&mut pending);
            }
        } else {
            let terminal = matches!(&reply,Reply::Event {event} if event.sequence==2);
            output.push(reply);
            if terminal {
                output.append(&mut plan.deferred.lock().unwrap());
            }
        }
    }
    output
}
pub(crate) async fn before_send(host: &str, reply: &Reply) -> bool {
    if let Reply::StreamAcknowledged {
        through_sequence, ..
    } = reply
    {
        pause_ack(host, AckCut::WorkerAfterCommit, *through_sequence).await;
    }
    let Some(plan) = plan(host, session(reply)) else {
        return true;
    };
    match reply {
        Reply::Event { event } if event.sequence == 1 => {
            plan.text_dropped.store(true, Ordering::Release);
            false
        }
        Reply::TurnAccepted { .. } if !plan.turn_ack_dropped.swap(true, Ordering::AcqRel) => false,
        Reply::StreamAcknowledged { .. } if !plan.ack_dropped.swap(true, Ordering::AcqRel) => false,
        Reply::StreamReplayed { events, .. } => {
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                while !plan.gap.load(Ordering::Acquire) {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("controller did not persist an actual Gap cut");
            if events.iter().any(|event| event.sequence == 1) {
                plan.replayed_text.store(true, Ordering::Release);
            }
            true
        }
        _ => true,
    }
}

/// Exact crash cuts for the separate process fixture. The private test runner
/// arms one cut for one host; the production binary cannot configure this.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AckCut {
    ControllerBeforeWire,
    WorkerAfterCommit,
}
impl AckCut {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ControllerBeforeWire => "controller_before_ack",
            Self::WorkerAfterCommit => "worker_after_ack",
        }
    }
}
static ACK_CUT: Mutex<Option<(String, AckCut)>> = Mutex::new(None);
pub(crate) struct AckCutGuard(String, AckCut);
impl AckCutGuard {
    pub(crate) fn install(host: &str, cut: AckCut) -> Self {
        let mut current = ACK_CUT.lock().unwrap();
        assert!(current.is_none());
        *current = Some((host.to_owned(), cut));
        Self(host.to_owned(), cut)
    }
}
impl Drop for AckCutGuard {
    fn drop(&mut self) {
        let mut current = ACK_CUT.lock().unwrap();
        if current
            .as_ref()
            .is_some_and(|(host, cut)| host == &self.0 && cut == &self.1)
        {
            current.take();
        }
    }
}
pub(crate) async fn pause_ack(host: &str, cut: AckCut, sequence: u64) {
    let selected = sequence == 1
        && ACK_CUT
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|(expected_host, expected_cut)| {
                expected_host == host && *expected_cut == cut
            });
    if selected {
        // Only a marker, never a receipt or invented SQL outcome. The parent
        // observes both real databases at this cut before issuing SIGKILL.
        println!("{}", serde_json::json!({"fixture_ack_cut":cut.label()}));
        std::future::pending::<()>().await;
    }
}
