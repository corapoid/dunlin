use super::*;
use crate::access::LiveAccess;
use std::{
    io::Write,
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Instant,
};
#[test]
fn guarded_executor_closes_header_body_tls_and_deadline_stalls_without_replay_or_partial_success() {
    for mode in ["headers", "body", "close-delimited", "tls", "deadline"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (started, ready) = mpsc::channel();
        let (closed, observed) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut bytes = [0; 1024];
            loop {
                let n = socket.read(&mut bytes).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&bytes[..n]);
                assert!(request.len() < 8192);
                if mode == "tls" || request.windows(4).any(|x| x == b"\r\n\r\n") {
                    break;
                }
            }
            if mode == "tls" {
                assert!(
                    !request
                        .windows(b"fixture-private-token".len())
                        .any(|x| x == b"fixture-private-token")
                );
            } else {
                assert!(String::from_utf8_lossy(&request).contains("Bearer fixture-private-token"));
            }
            if mode == "body" || mode == "deadline" {
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
                    .unwrap();
            }
            if mode == "close-delimited" {
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nx")
                    .unwrap();
            }
            started.send(()).unwrap();
            let result = socket.read(&mut bytes);
            assert!(
                matches!(result, Ok(0))
                    || matches!(result,Err(ref e) if matches!(e.kind(),std::io::ErrorKind::ConnectionReset|std::io::ErrorKind::BrokenPipe)),
                "peer did not close {mode}: {result:?}"
            );
            listener.set_nonblocking(true).unwrap();
            assert!(listener.accept().is_err(), "HTTP request was replayed");
            closed.send(()).unwrap();
        });
        // Controlled loopback transport fixture; production UreqExecutor::new
        // retains PublicResolver, TLS verification and its no-proxy policy.
        let executor = UreqExecutor {
            agent: ureq::AgentBuilder::new()
                .try_proxy_from_env(false)
                .redirects(0)
                .timeout(Duration::from_secs(10))
                .build(),
        };
        let granted = Arc::new(AtomicBool::new(true));
        let live = granted.clone();
        let duration = if mode == "deadline" {
            Duration::from_millis(300)
        } else {
            Duration::from_secs(10)
        };
        let access = LiveAccess::new(Instant::now() + duration, move || {
            live.load(Ordering::Acquire)
        });
        let plan = HttpPlan {
            method: "GET",
            url: format!(
                "{}://{address}/items",
                if mode == "tls" { "https" } else { "http" }
            ),
            headers: vec![(
                "Authorization".into(),
                "Bearer fixture-private-token".into(),
            )],
            body: None,
        };
        let client = thread::spawn(move || executor.execute_guarded(&plan, &access));
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        let revoked = Instant::now();
        if mode != "deadline" {
            granted.store(false, Ordering::Release);
        }
        observed.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            revoked.elapsed() < Duration::from_secs(1),
            "cancellation did not interrupt {mode}"
        );
        let error = client.join().unwrap().unwrap_err();
        assert!(!error.contains("fixture-private-token"));
        server.join().unwrap();
    }
}
#[test]
fn executor_without_explicit_cancellation_support_never_dispatches_a_guarded_request() {
    struct Unsupported(AtomicUsize);
    impl HttpExecutor for Unsupported {
        fn execute(&self, _: &HttpPlan) -> Result<HttpOutput, String> {
            self.0.fetch_add(1, Ordering::Relaxed);
            panic!("unguarded fallback must not run")
        }
    }
    let executor = Unsupported(AtomicUsize::new(0));
    let access = LiveAccess::new(Instant::now() + Duration::from_secs(1), || true);
    let plan = HttpPlan {
        method: "GET",
        url: "https://api.example.com/".into(),
        headers: vec![],
        body: None,
    };
    assert!(executor.execute_guarded(&plan, &access).is_err());
    assert_eq!(executor.0.load(Ordering::Relaxed), 0);
}
