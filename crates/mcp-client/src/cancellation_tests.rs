//! A peer holds a real HTTP response open until the client's socket is closed.
use super::*;
use std::{
    io::Write,
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};
fn read(socket: &mut std::net::TcpStream) -> Value {
    let mut reader = BufReader::new(socket);
    let mut length = 0;
    loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap();
        }
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
#[test]
fn revocation_closes_modern_and_legacy_headers_json_and_sse_without_replay_after_regrant() {
    for legacy in [false, true] {
        for framing in ["headers", "json", "sse"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/mcp", listener.local_addr().unwrap());
            let (started, ready) = mpsc::channel();
            let (closed, observed) = mpsc::channel();
            let server = thread::spawn(move || {
                let mut calls = 0;
                loop {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let request = read(&mut socket);
                    let method = request["method"].as_str().unwrap();
                    if method == "tools/call" {
                        calls += 1;
                        if framing != "headers" {
                            let content = if framing == "sse" {
                                "text/event-stream"
                            } else {
                                "application/json"
                            };
                            let partial = if framing == "sse" { "data: " } else { "{" };
                            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: {content}\r\nContent-Length: 4096\r\n\r\n{partial}").unwrap();
                        }
                        started.send(()).unwrap();
                        let mut b = [0; 1];
                        let result = socket.read(&mut b);
                        assert!(
                            matches!(result, Ok(0))
                                || matches!(result,Err(ref e) if e.kind()==std::io::ErrorKind::ConnectionReset),
                            "MCP peer did not close: {legacy}/{framing}: {result:?}"
                        );
                        listener.set_nonblocking(true).unwrap();
                        assert!(listener.accept().is_err());
                        closed.send(calls).unwrap();
                        break;
                    }
                    let result = match method {
                        "server/discover" => {
                            json!({"resultType":"complete","supportedVersions":[metadata::VERSION],"capabilities":{"tools":{}},"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"fixture","version":"1"}},"ttlMs":0,"cacheScope":"private"})
                        }
                        "initialize" => {
                            json!({"protocolVersion":VERSION,"capabilities":{"tools":{}}})
                        }
                        "notifications/initialized" => Value::Null,
                        "tools/list" => {
                            let tools = json!([{"name":"lookup","inputSchema":{"type":"object","properties":{}}}]);
                            if legacy {
                                json!({"tools":tools})
                            } else {
                                json!({"resultType":"complete","tools":tools,"ttlMs":0,"cacheScope":"private"})
                            }
                        }
                        other => panic!("unexpected MCP request {other}"),
                    };
                    let (status, body) = if method == "server/discover" && legacy {
                        (400, "legacy".into())
                    } else if method == "notifications/initialized" {
                        (202, String::new())
                    } else {
                        (
                            200,
                            json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string(),
                        )
                    };
                    write!(socket,"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                }
            });
            let granted = Arc::new(AtomicBool::new(true));
            let allowed = granted.clone();
            let client = thread::spawn(move || {
                let gate = allowed.clone();
                let guard = DispatchGuard::new(Arc::new(move || gate.load(Ordering::Acquire)));
                let mut client =
                    negotiated::Client::connect_authorized(&url, "fixture-private-token", guard)
                        .unwrap();
                let tool = client.tools().unwrap().remove(0);
                assert_eq!(client.call(&tool, json!({})), Err(Error::Unauthorized));
                allowed.store(true, Ordering::Release);
                assert_eq!(client.call(&tool, json!({})), Err(Error::Unauthorized));
                assert_eq!(client.tools().unwrap_err(), Error::Unauthorized);
            });
            ready.recv_timeout(Duration::from_secs(2)).unwrap();
            let revoked = Instant::now();
            granted.store(false, Ordering::Release);
            assert_eq!(observed.recv_timeout(Duration::from_secs(1)).unwrap(), 1);
            assert!(revoked.elapsed() < Duration::from_secs(1));
            client.join().unwrap();
            server.join().unwrap();
        }
    }
}

#[test]
fn subscription_peer_closes_on_revocation_or_owner_close_while_reader_stays_owned() {
    for owner_close in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let (closed, observed) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let request = read(&mut socket);
            assert_eq!(request["method"], "server/discover");
            let body=json!({"jsonrpc":"2.0","id":request["id"],"result":{"resultType":"complete","supportedVersions":[metadata::VERSION],"capabilities":{"tools":{}},"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"fixture","version":"1"}},"ttlMs":0,"cacheScope":"private"}}).to_string();
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            drop(socket);
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let request = read(&mut socket);
            assert_eq!(request["method"], "subscriptions/listen");
            let ack = json!({"jsonrpc":"2.0","method":"notifications/subscriptions/acknowledged","params":{"_meta":{"io.modelcontextprotocol/subscriptionId":request["id"]},"notifications":{"toolsListChanged":true}}});
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {ack}\n\n").unwrap();
            let mut byte = [0; 1];
            let result = socket.read(&mut byte);
            assert!(
                matches!(result, Ok(0))
                    || matches!(result,Err(ref e) if e.kind()==std::io::ErrorKind::ConnectionReset),
                "subscription socket not closed: {result:?}"
            );
            closed.send(()).unwrap();
        });
        let granted = Arc::new(AtomicBool::new(true));
        let live = granted.clone();
        let mut client = if owner_close {
            negotiated::Client::connect(&url, "fixture-token").unwrap()
        } else {
            negotiated::Client::connect_authorized(
                &url,
                "fixture-token",
                DispatchGuard::new(Arc::new(move || live.load(Ordering::Acquire))),
            )
            .unwrap()
        };
        let mut subscription = client
            .listen(negotiated::SubscriptionFilter {
                tools_list_changed: true,
                ..Default::default()
            })
            .unwrap();
        let (reading, ready) = mpsc::channel();
        let reader = thread::spawn(move || {
            reading.send(()).unwrap();
            subscription.next_event()
        });
        ready.recv_timeout(Duration::from_secs(1)).unwrap();
        let at = Instant::now();
        if owner_close {
            client.close().unwrap();
        } else {
            granted.store(false, Ordering::Release);
        }
        observed.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(at.elapsed() < Duration::from_secs(1));
        assert_eq!(
            reader.join().unwrap(),
            Err(if owner_close {
                Error::Protocol
            } else {
                Error::Unauthorized
            })
        );
        server.join().unwrap();
    }
}
