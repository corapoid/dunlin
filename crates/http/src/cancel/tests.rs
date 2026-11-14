use super::*;
use crate::{AgentBuilder, Resolver, config, http, unversioned};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    sync::mpsc,
    time::Instant,
};
#[derive(Debug)]
struct DelayedResolver {
    address: SocketAddr,
    started: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}
impl Resolver for DelayedResolver {
    fn resolve(
        &self,
        _: &http::Uri,
        _: &config::Config,
        _: unversioned::transport::NextTimeout,
    ) -> Result<unversioned::resolver::ResolvedSocketAddrs, crate::ResolverError> {
        self.started.send(()).unwrap();
        self.release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        let mut result = self.empty();
        result.push(self.address);
        Ok(result)
    }
}
#[test]
fn shutdown_during_dns_prevents_late_request_and_preserves_custom_resolver_in_isolated_agent() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let (started, ready) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let base = AgentBuilder::new()
        .try_proxy_from_env(false)
        .redirects(0)
        .timeout(Duration::from_secs(3))
        .resolver(DelayedResolver {
            address: listener.local_addr().unwrap(),
            started,
            release: Mutex::new(wait),
        })
        .build();
    let agent = base.isolated();
    let granted = Arc::new(AtomicBool::new(true));
    let live = granted.clone();
    let guard = agent.watch(move || live.load(Ordering::Acquire)).unwrap();
    let worker = thread::spawn(move || {
        agent
            .get("http://fixture.invalid/")
            .set("Authorization", "fixture-private")
            .call()
    });
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    granted.store(false, Ordering::Release);
    // finish joins the watchdog and synchronously observes denial before the
    // resolver is released, rather than relying on a timing-only race.
    assert!(!guard.finish());
    release.send(()).unwrap();
    assert!(worker.join().unwrap().is_err());
    // ureq can finish a pending TCP connect after shutdown, but cannot send
    // request bytes on it. A late accepted socket must be empty.
    match listener.accept() {
        Ok((mut socket, _)) => {
            socket
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut b = [0; 1];
            assert_eq!(socket.read(&mut b).unwrap(), 0);
        }
        Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::WouldBlock),
    }
}
#[test]
fn watcher_denial_is_terminal_and_panicking_authorizer_cancels_once() {
    let cancelled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = cancelled.clone();
    let result = Cancellation::new(
        || panic!("fixture authority panic"),
        move || {
            count.fetch_add(1, Ordering::AcqRel);
        },
    );
    assert!(result.is_err());
    assert_eq!(cancelled.load(Ordering::Acquire), 1);
    let granted = Arc::new(AtomicBool::new(true));
    let live = granted.clone();
    let (closed, received) = mpsc::channel();
    let guard = Cancellation::new(
        move || live.load(Ordering::Acquire),
        move || {
            closed.send(()).unwrap();
        },
    )
    .unwrap();
    granted.store(false, Ordering::Release);
    received.recv_timeout(Duration::from_secs(1)).unwrap();
    granted.store(true, Ordering::Release);
    assert!(!guard.finish());
    assert!(received.try_recv().is_err());
}
fn hold() -> (
    String,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (started, ready) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let worker = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut b = [0; 512];
        while !bytes.windows(4).any(|x| x == b"\r\n\r\n") {
            let n = socket.read(&mut b).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&b[..n]);
        }
        started.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(3)).unwrap();
        let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
    });
    (url, ready, release, worker)
}
#[test]
fn cancelling_one_isolated_pool_does_not_close_another_request() {
    let (url_a, ready_a, release_a, server_a) = hold();
    let (url_b, ready_b, release_b, server_b) = hold();
    let base = AgentBuilder::new()
        .try_proxy_from_env(false)
        .timeout(Duration::from_secs(10))
        .build();
    let a = base.isolated();
    let b = base.isolated();
    let granted = Arc::new(AtomicBool::new(true));
    let live = granted.clone();
    let guard = a.watch(move || live.load(Ordering::Acquire)).unwrap();
    let client_a = thread::spawn(move || a.get(&url_a).call());
    let client_b = thread::spawn(move || b.get(&url_b).call().unwrap().into_string().unwrap());
    ready_a.recv_timeout(Duration::from_secs(2)).unwrap();
    ready_b.recv_timeout(Duration::from_secs(2)).unwrap();
    let started = Instant::now();
    granted.store(false, Ordering::Release);
    assert!(client_a.join().unwrap().is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(!guard.finish());
    release_a.send(()).unwrap();
    release_b.send(()).unwrap();
    assert_eq!(client_b.join().unwrap(), "ok");
    server_a.join().unwrap();
    server_b.join().unwrap();
}
