use super::*;
use crate::DispatchGuard;
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};
fn accept(listener: &TcpListener) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(2))
            }
            result => panic!("missing discovery request: {result:?}"),
        }
    }
}
fn read(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut length = 0;
    loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        assert!(!line.to_ascii_lowercase().starts_with("authorization:"));
        if line == "\r\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':')
            && key.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().unwrap();
        }
    }
    assert!(length < 32768);
    reader.read_exact(&mut vec![0; length]).unwrap();
}
fn resource(origin: &str) -> ResourceMetadata {
    ResourceMetadata {
        resource: format!("{origin}/mcp"),
        authorization_servers: vec![format!("{origin}/issuer")],
        scopes_supported: None,
    }
}
#[test]
fn guarded_discovery_closes_probe_resource_and_issuer_io_without_publishing_after_regrant() {
    for target in ["probe", "resource", "issuer"] {
        for framing in ["headers", "body", "eof"] {
            if target == "probe" && framing != "headers" {
                continue;
            }
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let granted = Arc::new(AtomicBool::new(true));
            let allowed = granted.clone();
            let discovery = Discovery::new(&format!("{origin}/mcp"), &[])
                .unwrap()
                .with_guard(DispatchGuard::new(Arc::new(move || {
                    allowed.load(Ordering::Acquire)
                })));
            let body = if target == "resource" {
                json!({"resource":format!("{origin}/mcp"),"authorization_servers":[format!("{origin}/issuer")]})
            } else {
                json!({"issuer":format!("{origin}/issuer"),"authorization_endpoint":format!("{origin}/authorize"),"token_endpoint":format!("{origin}/token"),"response_types_supported":["code"],"code_challenge_methods_supported":["S256"]})
            }.to_string();
            let (started, ready) = mpsc::channel();
            let (closed, observed) = mpsc::channel();
            let peer = thread::spawn(move || {
                let mut stream = accept(&listener);
                read(&mut stream);
                if framing == "body" {
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4096\r\n\r\n{{").unwrap();
                } else if framing == "eof" {
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}").unwrap();
                }
                started.send(()).unwrap();
                let result = stream.read(&mut [0; 1]);
                assert!(
                    matches!(result, Ok(0))
                        || matches!(result,Err(ref e) if e.kind()==std::io::ErrorKind::ConnectionReset),
                    "discovery socket remained open: {result:?}"
                );
                closed.send(()).unwrap();
                assert!(listener.accept().is_err());
            });
            let regrant = granted.clone();
            let client = thread::spawn(move || {
                let attempt = || match target {
                    "probe" => discovery.protected_resource_from_server().map(|_| ()),
                    "resource" => discovery.protected_resource(None).map(|_| ()),
                    _ => discovery
                        .authorization_server(&resource(&origin), 0)
                        .map(|_| ()),
                };
                assert_eq!(attempt(), Err(Error::Unauthorized));
                regrant.store(true, Ordering::Release);
                assert_eq!(attempt(), Err(Error::Unauthorized));
            });
            ready.recv_timeout(Duration::from_secs(2)).unwrap();
            granted.store(false, Ordering::Release);
            observed.recv_timeout(Duration::from_secs(1)).unwrap();
            client.join().unwrap();
            peer.join().unwrap();
        }
    }
}
#[test]
fn revoked_discovery_never_tries_metadata_fallback_or_constructs_a_flow() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let granted = Arc::new(AtomicBool::new(true));
    let allowed = granted.clone();
    let discovery = Discovery::new(&format!("{origin}/mcp"), &[])
        .unwrap()
        .with_guard(DispatchGuard::new(Arc::new(move || {
            allowed.load(Ordering::Acquire)
        })));
    let peer = thread::spawn(move || {
        let mut stream = accept(&listener);
        read(&mut stream);
        granted.store(false, Ordering::Release);
        let _ = write!(
            stream,
            "HTTP/1.1 404 Missing\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        listener
    });
    assert!(matches!(
        discovery.protected_resource(None),
        Err(Error::Unauthorized)
    ));
    let listener = peer.join().unwrap();
    assert!(
        listener.accept().is_err(),
        "revoked attempt reached metadata fallback"
    );
    let metadata: AuthorizationMetadata = serde_json::from_value(json!({"issuer":format!("{origin}/issuer"),"authorization_endpoint":format!("{origin}/authorize"),"token_endpoint":format!("{origin}/token"),"response_types_supported":["code"],"code_challenge_methods_supported":["S256"]})).unwrap();
    let registration = Registration::public(
        &format!("{origin}/issuer"),
        "client",
        "http://127.0.0.1/callback",
    )
    .unwrap();
    assert!(matches!(
        discovery.authorization_flow(&resource(&origin), metadata, registration),
        Err(Error::Unauthorized)
    ));
}
