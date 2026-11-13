//! Local MCP delivery fixtures, including ambiguous mutations. No Linear token.
use super::*;
use dunlin_vault::store::{MemoryStore, SecretStore};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    thread,
};

fn mock(replies: Vec<(String, Value, u16)>) -> (String, thread::JoinHandle<Vec<Value>>) {
    mock_with_hook(replies, |_| {}, false)
}

fn mock_with_hook(
    mut replies: Vec<(String, Value, u16)>,
    mut before_reply: impl FnMut(&str) + Send + 'static,
    watch_unexpected: bool,
) -> (String, thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let mut requests = Vec::new();
        for (expected, result, status) in replies.drain(..) {
            let start = std::time::Instant::now();
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && start.elapsed() < Duration::from_secs(5) =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    _ => panic!("Missing status MCP request: {expected}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                request["params"]["name"]
                    .as_str()
                    .or(request["method"].as_str()),
                Some(expected.as_str())
            );
            before_reply(&expected);
            let body = if status == 202 {
                String::new()
            } else {
                json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string()
            };
            write!(socket,"HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            requests.push(request);
        }
        if watch_unexpected {
            // Keep the endpoint available after get_issue. A buggy implementation
            // must be caught sending update_issue, not merely hit a closed socket.
            let deadline = std::time::Instant::now() + Duration::from_millis(200);
            while std::time::Instant::now() < deadline {
                let mut socket = match listener.accept() {
                    Ok((socket, _)) => socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("Unexpected MCP listener error: {error}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(&mut socket);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                let body = json!({"jsonrpc":"2.0","id":request["id"],"result":{"structuredContent":{"success":true}}}).to_string();
                write!(socket,"HTTP/1.1 200 Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                requests.push(request);
            }
        }
        requests
    });
    (url, worker)
}

fn status_replies(
    record: &crate::store::jobs::Record,
    mutation: bool,
) -> Vec<(String, Value, u16)> {
    let mut replies = vec![
        (
            "initialize".into(),
            json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}}}),
            200,
        ),
        ("notifications/initialized".into(), Value::Null, 202),
        (
            "tools/list".into(),
            json!({"tools":[{"name":"get_issue","inputSchema":{"properties":{"id":{}}}},{"name":"update_issue","inputSchema":{"properties":{"id":{},"state":{}}}}]}),
            200,
        ),
        (
            "get_issue".into(),
            json!({"structuredContent":record.task.issue}),
            200,
        ),
    ];
    if mutation {
        replies.push((
            "update_issue".into(),
            json!({"structuredContent":{"success":true}}),
            200,
        ));
    }
    replies
}

fn attach_endpoint(f: &mut Fixture, endpoint: String) {
    let secrets = Arc::new(MemoryStore::new());
    secrets
        .set("fixture-credential", "fixture-only-linear-key")
        .unwrap();
    f.state.linear = Arc::new(crate::linear::Service::testing(secrets, endpoint));
}

#[test]
fn native_grant_revoke_regrant_during_status_read_sends_no_mutation_and_never_retries() {
    let mut f = Fixture::new("ok");
    let record = f.claim();
    let store = f.state.store.clone();
    let user = f.user.clone();
    let project = record.job.project_id.clone();
    let (endpoint, worker) = mock_with_hook(
        status_replies(&record, false),
        move |method| {
            if method == "get_issue" {
                let store = store.lock().unwrap();
                store.revoke_grant(&project, &user).unwrap();
                store.grant(&project, &user).unwrap();
            }
        },
        true,
    );
    attach_endpoint(&mut f, endpoint);
    let error = crate::linear::job_status(&f.state, &record).unwrap_err();
    assert!(
        !error.retry,
        "An obsolete approved epoch requires manual review"
    );
    let requests = worker.join().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r["params"]["name"] == "get_issue")
            .count(),
        1
    );
    assert!(
        !requests
            .iter()
            .any(|r| r["params"]["name"] == "update_issue")
    );
    let persisted = crate::store::Store::open(f.dir.path().join("relay.db"))
        .unwrap()
        .job(&record.job.id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.task.native_grant, record.task.native_grant);
    let current = f
        .state
        .store
        .lock()
        .unwrap()
        .capture_native_grant(&record.job.project_id, &f.user)
        .unwrap();
    assert_ne!(persisted.task.native_grant.as_ref().unwrap(), &current);
    assert!(
        !crate::linear::job_status(&f.state, &persisted)
            .unwrap_err()
            .retry,
        "Reloading an old persisted task cannot silently renew its consent"
    );
}

#[test]
fn persisted_job_from_previous_native_epoch_is_denied_before_any_mcp_request() {
    let mut f = Fixture::new("ok");
    let record = f.claim();
    {
        let store = f.state.store.lock().unwrap();
        store.revoke_grant(&record.job.project_id, &f.user).unwrap();
        store.grant(&record.job.project_id, &f.user).unwrap();
    }
    let persisted = crate::store::Store::open(f.dir.path().join("relay.db"))
        .unwrap()
        .job(&record.job.id)
        .unwrap()
        .unwrap();
    let (endpoint, worker) = mock_with_hook(Vec::new(), |_| {}, true);
    attach_endpoint(&mut f, endpoint);
    let error = crate::linear::job_status(&f.state, &persisted).unwrap_err();
    assert!(!error.retry);
    assert!(
        worker.join().unwrap().is_empty(),
        "A stale native epoch cannot even initialize external delivery"
    );
}

#[test]
fn status_mutation_completed_during_native_regrant_requires_manual_review_without_retry() {
    let mut f = Fixture::new("ok");
    let record = f.claim();
    let store = f.state.store.clone();
    let user = f.user.clone();
    let project = record.job.project_id.clone();
    let (endpoint, worker) = mock_with_hook(
        status_replies(&record, true),
        move |method| {
            if method == "update_issue" {
                let store = store.lock().unwrap();
                store.revoke_grant(&project, &user).unwrap();
                store.grant(&project, &user).unwrap();
            }
        },
        true,
    );
    attach_endpoint(&mut f, endpoint);
    let error = crate::linear::job_status(&f.state, &record).unwrap_err();
    assert!(
        !error.retry,
        "A response arriving after an epoch change is ambiguous, never automatically retried"
    );
    let requests = worker.join().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r["params"]["name"] == "update_issue")
            .count(),
        1
    );
    assert!(
        !crate::linear::job_status(&f.state, &record)
            .unwrap_err()
            .retry
    );
}

#[test]
fn status_delivery_rereads_state_and_never_retries_an_ambiguous_mutation() {
    for case in [
        "normal",
        "already_review",
        "human_changed",
        "label_removed",
        "read_failed",
        "mutation_failed",
    ] {
        let mut f = Fixture::new("ok");
        let record = f.claim();
        let mut issue = serde_json::to_value(&record.task.issue).unwrap();
        if case == "already_review" {
            issue["status"] = json!("In Review");
        }
        if case == "human_changed" {
            issue["status"] = json!("Done");
        }
        if case == "label_removed" {
            issue["labels"] = json!([]);
        }
        let mut replies = vec![
            (
                "initialize".into(),
                json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}}}),
                200,
            ),
            ("notifications/initialized".into(), Value::Null, 202),
            (
                "tools/list".into(),
                json!({"tools":[{"name":"get_issue","inputSchema":{"properties":{"id":{}}}},{"name":"update_issue","inputSchema":{"properties":{"id":{},"state":{}}}}]}),
                200,
            ),
            (
                "get_issue".into(),
                json!({"structuredContent":issue}),
                if case == "read_failed" { 503 } else { 200 },
            ),
        ];
        if matches!(case, "normal" | "mutation_failed") {
            replies.push((
                "update_issue".into(),
                json!({"structuredContent":{"success":true}}),
                if case == "mutation_failed" { 503 } else { 200 },
            ));
        }
        let (endpoint, worker) = mock(replies);
        let secrets = Arc::new(MemoryStore::new());
        secrets
            .set("fixture-credential", "fixture-only-linear-key")
            .unwrap();
        f.state.linear = Arc::new(crate::linear::Service::testing(secrets, endpoint));
        let result = crate::linear::job_status(&f.state, &record);
        match case {
            "normal" | "already_review" => assert!(result.is_ok(), "{case}"),
            "read_failed" => assert!(result.unwrap_err().retry),
            _ => assert!(!result.unwrap_err().retry, "{case}"),
        }
        let requests = worker.join().unwrap();
        let mutations: Vec<_> = requests
            .iter()
            .filter(|request| request["params"]["name"] == "update_issue")
            .collect();
        assert_eq!(
            mutations.len(),
            usize::from(matches!(case, "normal" | "mutation_failed"))
        );
        if let Some(request) = mutations.first() {
            assert_eq!(
                request["params"]["arguments"],
                json!({"id":"issue-1","state":"In Review"})
            );
        }
    }
}

#[test]
fn changed_linear_authorization_never_receives_a_previous_jobs_mutation() {
    let f = Fixture::new("ok");
    let record = f.claim();
    let store = f.state.store.lock().unwrap();
    let inbox = store.linear_connection(&f.user).unwrap().unwrap().inbox;
    store
        .linear_save(&f.user, "new-credential", "new-authorization", &inbox)
        .unwrap();
    drop(store);
    let error = crate::linear::job_status(&f.state, &record).unwrap_err();
    assert!(!error.retry);
    assert!(error.message.contains("authorization changed"));
}
