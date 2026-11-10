use super::*;
use std::{
    io::Write,
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

pub(crate) struct Reply {
    method: &'static str,
    result: Value,
    status: u16,
    sse: bool,
    session: bool,
}
impl Reply {
    pub fn initialize() -> Self {
        Self::result(
            "initialize",
            json!({"protocolVersion":VERSION,"capabilities":{"tools":{}}}),
        )
    }
    pub fn notification() -> Self {
        Self {
            status: 202,
            ..Self::result("notifications/initialized", Value::Null)
        }
    }
    pub fn result(method: &'static str, result: Value) -> Self {
        Self {
            method,
            result,
            status: 200,
            sse: false,
            session: false,
        }
    }
}
pub(crate) struct Request {
    pub body: Value,
    pub headers: String,
}
pub(crate) struct Mock {
    pub url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    worker: thread::JoinHandle<()>,
}
impl Mock {
    pub fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let worker = thread::spawn(move || {
            for reply in replies {
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
                        other => panic!("Missing mock request: {:?}", other.err()),
                    }
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(&mut socket);
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    headers.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                }
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                let body: Value = if bytes.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes).unwrap()
                };
                if reply.method == "DELETE" {
                    assert!(headers.starts_with("DELETE "));
                } else {
                    assert_eq!(body["method"], reply.method);
                }
                let message = json!({"jsonrpc":"2.0", "id":body["id"], "result":reply.result});
                let response_body = if reply.status == 202 || reply.status == 204 {
                    String::new()
                } else if reply.sse {
                    format!(
                        "data: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{{}}}}\r\n\r\ndata: {message}\r\n\r\n"
                    )
                } else {
                    message.to_string()
                };
                let content_type = if reply.sse {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let session = if reply.session {
                    "Mcp-Session-Id: fixture-session\r\n"
                } else {
                    ""
                };
                captured.lock().unwrap().push(Request { body, headers });
                write!(socket, "HTTP/1.1 {} Mock\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n{session}\r\n{response_body}", reply.status, response_body.len()).unwrap();
            }
        });
        Self {
            url,
            requests,
            worker,
        }
    }
    pub fn finish(self) -> Vec<Request> {
        self.worker.join().unwrap();
        Arc::try_unwrap(self.requests)
            .ok()
            .unwrap()
            .into_inner()
            .unwrap()
    }
}

#[test]
fn initializes_negotiates_session_and_reads_sse_tool_results() {
    let mock = Mock::new(vec![
        Reply {
            session: true,
            ..Reply::initialize()
        },
        Reply::notification(),
        Reply::result(
            "tools/list",
            json!({"tools":[{"name":"read","inputSchema":{"properties":{}}}]}),
        ),
        Reply {
            sse: true,
            ..Reply::result(
                "tools/call",
                json!({"content":[{"type":"text","text":"result"}]}),
            )
        },
        Reply {
            status: 204,
            ..Reply::result("DELETE", Value::Null)
        },
    ]);
    let mut client = Client::connect(&mock.url, "fixture-key").unwrap();
    let tools = client.tools().unwrap();
    assert_eq!(
        client.call(&tools[0], json!({})).unwrap()["content"][0]["text"],
        "result"
    );
    client.close().unwrap();
    let requests = mock.finish();
    for request in &requests[1..] {
        assert!(
            request
                .headers
                .to_ascii_lowercase()
                .contains("mcp-session-id: fixture-session")
        );
    }
    assert!(
        requests[0]
            .headers
            .to_ascii_lowercase()
            .contains("accept: application/json, text/event-stream")
    );
}

#[test]
fn pagination_and_duplicate_catalog_protection() {
    let mock = Mock::new(vec![
        Reply::initialize(),
        Reply::notification(),
        Reply::result("tools/list", json!({"tools":[],"nextCursor":"next"})),
        Reply::result(
            "tools/list",
            json!({"tools":[{"name":"read","inputSchema":{}}]}),
        ),
    ]);
    let mut client = Client::connect(&mock.url, "fixture-key").unwrap();
    assert_eq!(client.tools().unwrap().len(), 1);
    let requests = mock.finish();
    assert_eq!(requests[3].body["params"]["cursor"], "next");
}

#[test]
fn failed_tool_calls_and_expired_sessions_are_never_retried() {
    for status in [401, 404, 500] {
        let mock = Mock::new(vec![
            Reply {
                session: true,
                ..Reply::initialize()
            },
            Reply::notification(),
            Reply {
                status,
                ..Reply::result("tools/call", Value::Null)
            },
        ]);
        let mut client = Client::connect(&mock.url, "do-not-echo-this-key").unwrap();
        let tool = Tool {
            name: "mutate".into(),
            input_schema: json!({}),
        };
        let error = client.call(&tool, json!({})).unwrap_err();
        assert!(!error.to_string().contains("do-not-echo"));
        assert_eq!(mock.finish().len(), 3);
    }
}

#[test]
fn rpc_identity_errors_and_tool_failures_are_not_success() {
    assert_eq!(
        rpc_result(json!({"jsonrpc":"2.0","id":2,"result":{}}), 1).unwrap_err(),
        Error::Protocol
    );
    assert_eq!(
        rpc_result(
            json!({"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"secret"}}),
            1
        )
        .unwrap_err(),
        Error::Rpc(-1)
    );
    let mock = Mock::new(vec![
        Reply::initialize(),
        Reply::notification(),
        Reply::result(
            "tools/call",
            json!({"isError":true,"content":[{"type":"text","text":"secret"}]}),
        ),
    ]);
    let mut client = Client::connect(&mock.url, "fixture-key").unwrap();
    assert_eq!(
        client
            .call(
                &Tool {
                    name: "x".into(),
                    input_schema: json!({})
                },
                json!({})
            )
            .unwrap_err(),
        Error::ToolFailed
    );
    mock.finish();
}

#[test]
fn endpoints_and_credentials_fail_before_network_access() {
    for endpoint in [
        "http://example.com/mcp",
        "https://user:pass@example.com/mcp",
        "https://example.com/mcp?token=x",
        "https://example.com/mcp#x",
        "file:///tmp/mcp",
    ] {
        assert_eq!(validate_endpoint(endpoint), Err(Error::Endpoint));
    }
    assert!(validate_endpoint("https://mcp.linear.app/mcp").is_ok());
    assert!(matches!(
        Client::connect("http://127.0.0.1:1/mcp", "secret\r\nx:y"),
        Err(Error::Credential)
    ));
}

#[test]
fn redirects_duplicate_tools_and_repeated_cursors_fail_closed() {
    let mock = Mock::new(vec![Reply {
        status: 307,
        ..Reply::initialize()
    }]);
    assert!(matches!(
        Client::connect(&mock.url, "fixture-key"),
        Err(Error::Http(307))
    ));
    assert_eq!(mock.finish().len(), 1);
    let mock = Mock::new(vec![
        Reply::initialize(),
        Reply::notification(),
        Reply::result(
            "tools/list",
            json!({"tools":[{"name":"same","inputSchema":{}},{"name":"same","inputSchema":{}}]}),
        ),
    ]);
    let mut client = Client::connect(&mock.url, "fixture-key").unwrap();
    assert!(matches!(client.tools(), Err(Error::Protocol)));
    mock.finish();
    let mock = Mock::new(vec![
        Reply::initialize(),
        Reply::notification(),
        Reply::result("tools/list", json!({"tools":[],"nextCursor":"same"})),
        Reply::result("tools/list", json!({"tools":[],"nextCursor":"same"})),
    ]);
    let mut client = Client::connect(&mock.url, "fixture-key").unwrap();
    assert!(matches!(client.tools(), Err(Error::Protocol)));
    mock.finish();
}
