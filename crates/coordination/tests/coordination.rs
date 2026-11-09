use dunlin_coordination::service::{Service, SessionHandle, request};
use dunlin_coordination::*;
use serde_json::{Value, json};

fn scope(project: &str) -> Scope {
    Scope {
        authority: "local".into(),
        project: project.into(),
    }
}
fn run(session: &str, project: &str) -> Run {
    Run {
        id: new_id(),
        session: session.into(),
        scope: scope(project),
        execution: Execution {
            root: "/repo".into(),
            harness: "Mock".into(),
            model: "mock-model".into(),
            mode: "plan".into(),
            permission: "Ask".into(),
            configuration: "digest".into(),
        },
        title: session.into(),
        status: RunStatus::Running,
        queued_prompts: Some(0),
        created_ms: now_ms(),
        updated_ms: now_ms(),
        stop_reason: None,
        report: None,
        continuation: None,
    }
}
struct Fixture {
    _dir: tempfile::TempDir,
    service: Service,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let service = Service::start(dir.path()).unwrap();
        Self { _dir: dir, service }
    }
    fn start(&self, session: &str, project: &str) -> (Run, SessionHandle) {
        let r = run(session, project);
        self.service
            .store
            .lock()
            .unwrap()
            .start_run(r.clone())
            .unwrap();
        let h = self
            .service
            .grant(scope(project), session.into(), "/repo".into())
            .unwrap();
        (r, h)
    }
    fn call(&self, h: &SessionHandle, name: &str, args: Value) -> Result<Value, String> {
        request(self.service.socket(), &h.token, name, args)
    }
    fn propose(&self, h: &SessionHandle, deps: Option<Vec<Dependency>>) -> Continuation {
        let mut args =
            json!({"prompt":"Implement after dependencies","idempotency_key":"defer-once"});
        if let Some(deps) = deps {
            args["dependencies"] = json!(deps);
        }
        serde_json::from_value(self.call(h, "continuations_create", args).unwrap()).unwrap()
    }
    fn finish(&self, r: &Run, reason: &str) {
        self.service
            .store
            .lock()
            .unwrap()
            .finish_run(&r.id, RunStatus::from_stop(reason), reason, Some(0), None)
            .unwrap();
    }
}

#[test]
fn both_finish_orders_release_only_one_reserved_turn_and_preserve_plan_mode() {
    for reverse in [false, true] {
        let f = Fixture::new();
        let (a, _) = f.start("a", "p");
        let (b, _) = f.start("b", "p");
        let (c, h) = f.start("c", "p");
        let proposed = f.propose(&h, None);
        assert_eq!(proposed.dependencies.len(), 2);
        let mut db = f.service.store.lock().unwrap();
        db.approve(&proposed.id, &c.execution, now_ms() + 60_000)
            .unwrap();
        drop(db);
        f.finish(&c, "end_turn");
        let (first, last) = if reverse { (&b, &a) } else { (&a, &b) };
        f.finish(first, "end_turn");
        {
            let db = f.service.store.lock().unwrap();
            assert!(
                !db.ledger()
                    .readiness(&db.ledger().continuations[0], now_ms())
                    .unwrap()
            );
        }
        f.finish(last, "end_turn");
        let mut next = run("c", "p");
        next.continuation = Some(proposed.id.clone());
        let mut db = f.service.store.lock().unwrap();
        let mut changed = next.clone();
        changed.execution.mode = "build".into();
        assert!(db.reserve_continuation(&proposed.id, changed).is_err());
        db.reserve_continuation(&proposed.id, next.clone()).unwrap();
        assert!(db.reserve_continuation(&proposed.id, next.clone()).is_err());
        db.confirm_dispatch(&proposed.id).unwrap();
        assert_eq!(
            db.ledger().continuations[0].status,
            ContinuationStatus::Dispatched
        );
        assert_eq!(db.ledger().runs.last().unwrap().execution.mode, "plan");
    }
}

#[test]
fn restart_keeps_history_but_never_replays_an_uncertain_wire_send() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.sqlite");
    let mut db = Store::open(&path).unwrap();
    let a = run("a", "p");
    let c = run("c", "p");
    db.start_run(a.clone()).unwrap();
    db.start_run(c.clone()).unwrap();
    let proposal = Continuation {
        id: new_id(),
        scope: scope("p"),
        session: "c".into(),
        requester_run: c.id.clone(),
        execution: c.execution.clone(),
        prompt: "Future task".into(),
        dependencies: vec![Dependency::Run(a.id.clone())],
        status: ContinuationStatus::PendingApproval,
        created_ms: now_ms(),
        deadline_ms: now_ms() + 60_000,
        approved_ms: None,
        reason: None,
        dispatched_run: None,
        executor_device: None,
        executor_connection: None,
        key: "once".into(),
    };
    db.mutate(|l| {
        l.continuations.push(proposal.clone());
        Ok(())
    })
    .unwrap();
    db.approve(&proposal.id, &c.execution, now_ms() + 60_000)
        .unwrap();
    for r in [&a, &c] {
        db.finish_run(&r.id, RunStatus::TurnFinished, "end_turn", Some(0), None)
            .unwrap();
    }
    let mut next = run("c", "p");
    next.continuation = Some(proposal.id.clone());
    db.reserve_continuation(&proposal.id, next).unwrap();
    drop(db);
    let mut db = Store::open(&path).unwrap();
    db.recover().unwrap();
    assert_eq!(
        db.ledger().continuations[0].status,
        ContinuationStatus::NeedsAttention
    );
    assert_eq!(db.ledger().runs.last().unwrap().status, RunStatus::Unknown);
    assert!(
        db.approve(&proposal.id, &c.execution, now_ms() + 60_000)
            .is_err()
    );
}

#[test]
fn task_completion_is_a_claim_and_still_waits_for_reporting_turn() {
    let f = Fixture::new();
    let (a, ha) = f.start("a", "p");
    let (c, hc) = f.start("c", "p");
    let task = f
        .call(
            &ha,
            "tasks_create",
            json!({"title":"Multi-turn task","idempotency_key":"task"}),
        )
        .unwrap();
    let task_id = task["id"].as_str().unwrap().to_owned();
    let continuation = f.propose(&hc, Some(vec![Dependency::Task(task_id.clone())]));
    f.finish(&c, "end_turn");
    f.finish(&a, "end_turn");
    {
        let db = f.service.store.lock().unwrap();
        assert!(!db.ledger().readiness(&continuation, now_ms()).unwrap());
    }
    let next = run("a", "p");
    f.service
        .store
        .lock()
        .unwrap()
        .start_run(next.clone())
        .unwrap();
    f.call(&ha,"tasks_report",json!({"id":task_id,"succeeded":true,"report":{"summary":"Done","evidence":["tests/output.txt"]}})).unwrap();
    {
        let db = f.service.store.lock().unwrap();
        assert!(!db.ledger().readiness(&continuation, now_ms()).unwrap());
    }
    f.finish(&next, "end_turn");
    let db = f.service.store.lock().unwrap();
    assert!(db.ledger().readiness(&continuation, now_ms()).unwrap());
    assert_eq!(db.ledger().tasks[0].runs.len(), 2);
}

#[test]
fn blockers_never_become_task_success_and_foreign_sessions_cannot_report() {
    let f = Fixture::new();
    let (_, a) = f.start("a", "p");
    let (_, b) = f.start("b", "p");
    let task = f
        .call(
            &a,
            "tasks_create",
            json!({"title":"Task","idempotency_key":"task"}),
        )
        .unwrap();
    let args = json!({"id":task["id"],"succeeded":true,"report":{"summary":"Blocked","blockers":["Tests fail"]}});
    assert!(f.call(&b, "tasks_report", args.clone()).is_err());
    f.call(&a, "tasks_report", args).unwrap();
    assert_eq!(
        f.service.store.lock().unwrap().ledger().tasks[0].status,
        TaskStatus::ReportedFailed
    );
}

#[test]
fn self_dependencies_cycles_and_cross_project_reads_are_refused() {
    let f = Fixture::new();
    let (a, ha) = f.start("a", "p");
    let (b, hb) = f.start("b", "p");
    let (other, ho) = f.start("other", "other");
    assert!(f.call(&ha,"continuations_create",json!({"prompt":"Self","idempotency_key":"self","dependencies":[{"kind":"run","id":a.id}]})).is_err());
    f.propose(&ha, Some(vec![Dependency::Run(b.id.clone())]));
    assert!(f.call(&hb,"continuations_create",json!({"prompt":"Cycle","idempotency_key":"cycle","dependencies":[{"kind":"run","id":a.id}]})).is_err());
    assert!(f.call(&ho, "runs_get", json!({"id":a.id})).is_err());
    assert!(f.call(&ha, "runs_get", json!({"id":other.id})).is_err());
    assert!(
        f.call(&ha, "agents_list", json!({"project":"other"}))
            .is_err()
    );
}

#[test]
fn conversation_reuse_cannot_change_a_dependency_or_reactivate_a_proposal() {
    let f = Fixture::new();
    let (a, _) = f.start("a", "p");
    let (c, hc) = f.start("c", "p");
    let proposal = f.propose(&hc, None);
    f.finish(&a, "end_turn");
    f.finish(&c, "end_turn");
    let later = run("a", "p");
    f.service.store.lock().unwrap().start_run(later).unwrap();
    {
        let db = f.service.store.lock().unwrap();
        assert!(db.ledger().readiness(&proposal, now_ms()).unwrap());
    }
    let later = run("c", "p");
    f.service.store.lock().unwrap().start_run(later).unwrap();
    assert!(
        f.service
            .store
            .lock()
            .unwrap()
            .ledger()
            .readiness(&proposal, now_ms())
            .is_err()
    );
}

#[test]
fn idempotency_conflicts_limits_and_cancellation_are_durable() {
    let f = Fixture::new();
    let (_, a) = f.start("a", "p");
    let (_, b) = f.start("b", "p");
    let m = f
        .call(
            &a,
            "messages_send",
            json!({"to":"b","body":"Hello","idempotency_key":"m"}),
        )
        .unwrap();
    assert!(
        f.call(
            &a,
            "messages_send",
            json!({"to":"b","body":"Different","idempotency_key":"m"})
        )
        .is_err()
    );
    assert!(f.call(&a, "inbox_ack", json!({"id":m["id"]})).is_err());
    f.call(&b, "inbox_ack", json!({"id":m["id"]})).unwrap();
    assert!(
        f.call(
            &a,
            "messages_send",
            json!({"to":"b","body":"x".repeat(MAX_TEXT+1),"idempotency_key":"big"})
        )
        .is_err()
    );
    let c = f.propose(&a, None);
    f.call(&a, "continuations_cancel", json!({"id":c.id}))
        .unwrap();
    let mut db = f.service.store.lock().unwrap();
    assert_eq!(
        db.ledger().continuations[0].status,
        ContinuationStatus::Cancelled
    );
    assert!(db.approve(&c.id, &c.execution, now_ms() + 60_000).is_err());
}

#[test]
fn live_authorization_is_checked_per_call_not_only_at_handshake() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let f = Fixture::new();
    let allowed = Arc::new(AtomicBool::new(true));
    let check = allowed.clone();
    let h = f
        .service
        .grant_guarded(
            scope("p"),
            "s".into(),
            "/repo".into(),
            Some(Arc::new(move || check.load(Ordering::SeqCst))),
        )
        .unwrap();
    assert!(f.call(&h, "agents_list", json!({})).is_ok());
    allowed.store(false, Ordering::SeqCst);
    assert!(f.call(&h, "agents_list", json!({})).is_err());
}

#[test]
fn corrupt_or_newer_storage_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger.sqlite");
    drop(Store::open(&path).unwrap());
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "INSERT INTO coordination_ledger(id,version,data) VALUES(1,99,'{}')",
            [],
        )
        .unwrap();
    assert!(Store::open(&path).is_err());
    let version: u32 = connection
        .query_row("SELECT version FROM coordination_ledger", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 99);
}

#[test]
fn archive_is_scoped_and_retains_live_dependency_evidence() {
    let f = Fixture::new();
    let (a, _) = f.start("a", "p");
    let (c, h) = f.start("c", "p");
    let (other, _) = f.start("other", "other");
    f.propose(&h, None);
    f.finish(&a, "end_turn");
    f.finish(&c, "end_turn");
    f.finish(&other, "end_turn");
    let mut db = f.service.store.lock().unwrap();
    db.archive_scope(Some(&scope("p"))).unwrap();
    assert_eq!(db.ledger().continuations.len(), 1);
    assert_eq!(db.ledger().runs.len(), 3);
    db.ledger().validate().unwrap();
}

#[test]
fn expiry_and_backwards_clock_never_release_work() {
    let f = Fixture::new();
    let (c, h) = f.start("c", "p");
    let proposal = f.propose(&h, None);
    f.finish(&c, "end_turn");
    let db = f.service.store.lock().unwrap();
    assert!(
        db.ledger()
            .readiness(&proposal, proposal.created_ms.saturating_sub(1))
            .is_err()
    );
    assert!(
        db.ledger()
            .readiness(&proposal, proposal.deadline_ms)
            .is_err()
    );
}

#[test]
fn a_failure_is_visible_even_while_other_dependencies_are_still_active() {
    let f = Fixture::new();
    let (_, _) = f.start("still-running", "p");
    let (failed, _) = f.start("failed", "p");
    let (c, h) = f.start("c", "p");
    let proposal = f.propose(&h, None);
    f.finish(&c, "end_turn");
    f.finish(&failed, "refusal");
    assert!(
        f.service
            .store
            .lock()
            .unwrap()
            .ledger()
            .readiness(&proposal, now_ms())
            .is_err()
    );
}
