//! Positive token operations are cancelled through real response-body I/O.
use super::*;
use crate::DispatchGuard;
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

fn flow(origin: &str) -> AuthorizationFlow {
    let discovery = Discovery::new(&format!("{origin}/mcp"), &[]).unwrap();
    discovery
        .authorization_flow(
            &ResourceMetadata {
                resource: format!("{origin}/mcp"),
                authorization_servers: vec![format!("{origin}/issuer")],
                scopes_supported: None,
            },
            tests::metadata(origin, true),
            tests::registration(origin, "public"),
        )
        .unwrap()
}

#[test]
fn token_exchange_and_refresh_close_stalled_headers_body_and_clean_eof_without_accepting_or_replaying()
 {
    for refresh in [false, true] {
        for framing in ["headers", "body", "eof"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let granted = Arc::new(AtomicBool::new(true));
            let check = granted.clone();
            let flow = flow(&origin).with_token_guard(DispatchGuard::new(Arc::new(move || {
                check.load(Ordering::Acquire)
            })));
            let request = flow
                .begin(tests::context(), vec!["files:read".into()])
                .unwrap();
            let callback = tests::callback(&flow, &request, Some(&format!("{origin}/issuer")));
            let (started, ready) = mpsc::channel();
            let (closed, observed) = mpsc::channel();
            let peer = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(&mut socket);
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
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                assert!((1..32768).contains(&length));
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let form = url::form_urlencoded::parse(&body).collect::<BTreeMap<_, _>>();
                assert_eq!(
                    form["grant_type"],
                    if refresh {
                        "refresh_token"
                    } else {
                        "authorization_code"
                    }
                );
                if framing == "body" {
                    write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4096\r\n\r\n{{").unwrap();
                } else if framing == "eof" {
                    // Syntactically complete tokens, but no EOF until shutdown.
                    write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}", tests::tokens()).unwrap();
                }
                started.send(()).unwrap();
                let result = socket.read(&mut [0; 1]);
                assert!(
                    matches!(result, Ok(0))
                        || matches!(result, Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionReset),
                    "token socket remained open: {result:?}"
                );
                closed.send(()).unwrap();
                listener.set_nonblocking(true).unwrap();
                assert!(listener.accept().is_err(), "token operation replayed");
            });
            let retry = granted.clone();
            let client = thread::spawn(move || {
                if refresh {
                    let mut token = TokenSet {
                        context: tests::context(),
                        endpoint: flow.endpoint.clone(),
                        issuer: flow.issuer.clone(),
                        client_id: flow.registration.client_id.clone(),
                        resource: flow.resource.clone(),
                        access_token: Secret(Zeroizing::new("old-access".into())),
                        refresh_token: Some(Secret(Zeroizing::new("old-refresh".into()))),
                        scopes: vec!["files:read".into()],
                        expires: Instant::now(),
                        status: CredentialStatus::Active,
                    };
                    assert_eq!(
                        flow.refresh(&tests::context(), &mut token),
                        Err(Error::Unauthorized)
                    );
                    assert_eq!(token.status(), CredentialStatus::RefreshUncertain);
                    assert_eq!(token.access_token.0.as_str(), "old-access");
                    retry.store(true, Ordering::Release);
                    assert_eq!(
                        flow.refresh(&tests::context(), &mut token),
                        Err(Error::Unauthorized)
                    );
                } else {
                    assert!(matches!(
                        flow.finish(&tests::context(), &callback),
                        Err(Error::Unauthorized)
                    ));
                    retry.store(true, Ordering::Release);
                    assert!(matches!(
                        flow.finish(&tests::context(), &callback),
                        Err(Error::Unauthorized)
                    ));
                }
            });
            ready.recv_timeout(Duration::from_secs(2)).unwrap();
            granted.store(false, Ordering::Release);
            observed.recv_timeout(Duration::from_secs(1)).unwrap();
            client.join().unwrap();
            peer.join().unwrap();
        }
    }
}
