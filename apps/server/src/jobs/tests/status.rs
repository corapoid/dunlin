//! Local MCP delivery fixtures, including ambiguous mutations. No Linear token.
use super::*;
use dunlin_vault::store::{MemoryStore, SecretStore};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    thread,
};

fn mock(mut replies: Vec<(String, Value, u16)>) -> (String, thread::JoinHandle<Vec<Value>>) {
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
                if let Some((name, value)) = line.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
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
            let body = if status == 202 {
                String::new()
            } else {
                json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string()
            };
            write!(socket,"HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            requests.push(request);
        }
        requests
    });
    (url, worker)
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
