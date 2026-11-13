//! Synchronous HTTP on ureq 3, preserving status bodies and per-request policy.
//!
//! ureq 3's `StatusCode` error discards the response. Dunlin needs that body for
//! relay diagnostics, but must keep it out of provider/vault errors. Keep this
//! application-facing boundary instead of duplicating policy at every call site.
//! No retries are added here, especially for remote mutations.
use std::{fmt, io::Read, sync::Arc, time::Duration};
use ureq::RequestExt;

pub use ureq::Error as ResolverError;
pub use ureq::unversioned::resolver::Resolver;
pub use ureq::{config, http, unversioned};

#[derive(Clone)]
pub struct Agent(ureq::Agent);

pub struct AgentBuilder {
    config: config::ConfigBuilder<ureq::typestate::AgentScope>,
    resolver: Option<SharedResolver>,
}

#[derive(Clone)]
struct SharedResolver(Arc<dyn Resolver>);

impl fmt::Debug for SharedResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DunlinResolver")
    }
}

impl Resolver for SharedResolver {
    fn resolve(
        &self,
        uri: &http::Uri,
        config: &config::Config,
        timeout: unversioned::transport::NextTimeout,
    ) -> Result<unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        self.0.resolve(uri, config, timeout)
    }
}

impl Default for AgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentBuilder {
    pub fn new() -> Self {
        Self {
            config: ureq::Agent::config_builder()
                .http_status_as_error(false)
                .max_redirects(5)
                .max_redirects_will_error(true),
            resolver: None,
        }
    }

    pub fn try_proxy_from_env(mut self, enabled: bool) -> Self {
        if !enabled {
            self.config = self.config.proxy(None);
        }
        self
    }

    pub fn redirects(mut self, count: u32) -> Self {
        self.config = self
            .config
            .max_redirects(count)
            .max_redirects_will_error(count > 0);
        self
    }

    /// Explicit per-agent CA for private relays. No system trust changes and no
    /// disabling of certificate or hostname verification.
    pub fn root_certificate_pem(mut self, pem: &[u8]) -> Result<Self, String> {
        let certificate = ureq::tls::Certificate::from_pem(pem)
            .map_err(|_| "Invalid relay CA PEM".to_string())?;
        self.config = self.config.tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::Specific(vec![certificate].into()))
                .build(),
        );
        Ok(self)
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config = self.config.timeout_global(Some(timeout));
        self
    }

    pub fn timeout_connect(mut self, timeout: Duration) -> Self {
        self.config = self.config.timeout_connect(Some(timeout));
        self
    }

    pub fn resolver(mut self, resolver: impl Resolver) -> Self {
        self.resolver = Some(SharedResolver(Arc::new(resolver)));
        self
    }

    pub fn build(self) -> Agent {
        let config = self.config.build();
        Agent(match self.resolver {
            Some(resolver) => ureq::Agent::with_parts(
                config,
                unversioned::transport::DefaultConnector::default(),
                resolver,
            ),
            None => config.new_agent(),
        })
    }
}

impl Agent {
    pub fn request(&self, method: &str, url: &str) -> Request {
        Request {
            agent: self.clone(),
            builder: http::Request::builder().method(method).uri(url),
            timeout: None,
            content_type: false,
        }
    }

    pub fn get(&self, url: &str) -> Request {
        self.request("GET", url)
    }
    pub fn post(&self, url: &str) -> Request {
        self.request("POST", url)
    }
    pub fn put(&self, url: &str) -> Request {
        self.request("PUT", url)
    }
    pub fn delete(&self, url: &str) -> Request {
        self.request("DELETE", url)
    }
}

pub struct Request {
    agent: Agent,
    builder: http::request::Builder,
    timeout: Option<Duration>,
    content_type: bool,
}

impl Request {
    pub fn set(mut self, name: &str, value: &str) -> Self {
        self.content_type |= name.eq_ignore_ascii_case("content-type");
        if let Some(headers) = self.builder.headers_mut() {
            headers.remove(name);
        }
        self.builder = self.builder.header(name, value);
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn call(self) -> Result<Response, Error> {
        self.send_bytes(&[])
    }
    pub fn send_string(self, body: &str) -> Result<Response, Error> {
        self.send_bytes(body.as_bytes())
    }

    pub fn send_json(mut self, body: impl serde::Serialize) -> Result<Response, Error> {
        if !self.content_type {
            self = self.set("Content-Type", "application/json");
        }
        let body = serde_json::to_vec(&body)
            .map_err(|error| Error::Transport(Transport(ureq::Error::Json(error))))?;
        self.send_bytes(&body)
    }

    pub fn send_bytes(self, body: &[u8]) -> Result<Response, Error> {
        let request = self
            .builder
            .body(body)
            .map_err(|error| Error::Transport(Transport(ureq::Error::Http(error))))?;
        let request = request.with_agent(&self.agent.0);
        let response = match self.timeout {
            Some(timeout) => request.configure().timeout_global(Some(timeout)).run(),
            None => request.run(),
        }
        .map_err(|error| Error::Transport(Transport(error)))?;
        let response = Response(response);
        if response.status() >= 400 {
            Err(Error::Status(response.status(), Box::new(response)))
        } else {
            Ok(response)
        }
    }
}

#[derive(Debug)]
pub struct Response(http::Response<ureq::Body>);

impl Response {
    pub fn new(status: u16, _reason: &str, body: &str) -> Result<Self, http::Error> {
        http::Response::builder()
            .status(status)
            .body(ureq::Body::builder().data(body.as_bytes().to_vec()))
            .map(Self)
    }

    pub fn status(&self) -> u16 {
        self.0.status().as_u16()
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.0.headers().get(name)?.to_str().ok()
    }
    /// Preserve repeated challenge fields and report invalid header text instead
    /// of silently dropping it while selecting an authorization mechanism.
    pub fn header_values(
        &self,
        name: &str,
    ) -> impl Iterator<Item = Result<&str, http::header::ToStrError>> {
        self.0
            .headers()
            .get_all(name)
            .iter()
            .map(|value| value.to_str())
    }
    pub fn into_reader(self) -> impl Read + Send + 'static {
        self.0.into_body().into_reader()
    }
    pub fn into_json<T: serde::de::DeserializeOwned>(self) -> Result<T, serde_json::Error> {
        // Size policies belong to each endpoint. In particular, relay session
        // transcripts must not acquire ureq 3's default 10 MiB JSON limit.
        serde_json::from_reader(self.into_reader())
    }
    pub fn into_string(self) -> Result<String, ureq::Error> {
        self.0.into_body().read_to_string()
    }
}

#[derive(Debug)]
pub enum Error {
    Status(u16, Box<Response>),
    Transport(Transport),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(status, _) => write!(f, "HTTP {status}"),
            Self::Transport(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for Error {}

#[derive(Debug)]
pub struct Transport(ureq::Error);

impl fmt::Display for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Copy)]
pub enum ErrorKind {
    Dns,
    ConnectionFailed,
    Io,
    InsecureRequestHttpsOnly,
    TooManyRedirects,
    InvalidUrl,
    UnknownScheme,
    Other,
}

impl Transport {
    pub fn kind(&self) -> ErrorKind {
        match self.0 {
            ureq::Error::HostNotFound => ErrorKind::Dns,
            ureq::Error::ConnectionFailed => ErrorKind::ConnectionFailed,
            ureq::Error::Io(_) => ErrorKind::Io,
            ureq::Error::RequireHttpsOnly(_) => ErrorKind::InsecureRequestHttpsOnly,
            ureq::Error::TooManyRedirects => ErrorKind::TooManyRedirects,
            ureq::Error::BadUri(_) | ureq::Error::Http(_) => ErrorKind::InvalidUrl,
            _ => ErrorKind::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, sync::mpsc, thread};

    fn server(response: String) -> (String, mpsc::Receiver<Vec<u8>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            tx.send(bytes).unwrap();
            stream.write_all(response.as_bytes()).unwrap();
        });
        (url, rx, worker)
    }

    fn agent() -> Agent {
        AgentBuilder::new()
            .try_proxy_from_env(false)
            .redirects(0)
            .timeout(Duration::from_secs(3))
            .build()
    }

    #[test]
    fn get_and_json_posts_preserve_headers_body_and_per_request_timeout() {
        let (url, received, worker) = server("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}".into());
        let response = agent()
            .post(&url)
            .set("Authorization", "old")
            .set("Authorization", "Bearer fixture")
            .timeout(Duration::from_secs(2))
            .send_json(serde_json::json!({"name":"zażółć"}))
            .unwrap();
        assert_eq!(response.header("content-type"), Some("application/json"));
        assert_eq!(
            response.into_json::<serde_json::Value>().unwrap(),
            serde_json::json!({"ok":true})
        );
        let request = String::from_utf8(received.recv().unwrap()).unwrap();
        assert!(request.starts_with("POST / HTTP/1.1"));
        assert_eq!(
            request
                .to_ascii_lowercase()
                .matches("authorization:")
                .count(),
            1
        );
        assert!(request.contains("Bearer fixture"));
        assert!(request.contains("application/json"));
        assert!(request.ends_with("{\"name\":\"zażółć\"}"));
        worker.join().unwrap();

        let (url, received, worker) =
            server("HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".into());
        assert_eq!(agent().get(&url).call().unwrap().status(), 204);
        assert!(received.recv().unwrap().starts_with(b"GET / HTTP/1.1"));
        worker.join().unwrap();
    }

    #[test]
    fn status_errors_keep_the_response_without_exposing_its_body_in_display() {
        let (url, received, worker) = server(
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 6\r\nConnection: close\r\n\r\nsecret".into(),
        );
        let error = agent().get(&url).call().unwrap_err();
        assert_eq!(error.to_string(), "HTTP 403");
        let Error::Status(403, response) = error else {
            panic!("expected HTTP status error")
        };
        assert_eq!(response.into_string().unwrap(), "secret");
        received.recv().unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn repeated_authentication_challenges_preserve_order_and_invalid_text_is_reported() {
        for (fields, invalid) in [
            (
                "WWW-Authenticate: Basic realm=other\r\nWWW-Authenticate: Bearer scope=read\r\n",
                false,
            ),
            ("WWW-Authenticate: Bearer realm=\"é\"\r\n", true),
        ] {
            let (url, request, worker) = server(format!(
                "HTTP/1.1 401 Unauthorized\r\n{fields}Content-Length: 0\r\nConnection: close\r\n\r\n"
            ));
            let response = match agent().get(&url).call() {
                Err(Error::Status(401, response)) => response,
                _ => panic!("401 expected"),
            };
            let values = response
                .header_values("WWW-Authenticate")
                .collect::<Result<Vec<_>, _>>();
            if invalid {
                assert!(values.is_err());
            } else {
                assert_eq!(
                    values.unwrap(),
                    vec!["Basic realm=other", "Bearer scope=read"]
                );
            }
            request.recv_timeout(Duration::from_secs(3)).unwrap();
            worker.join().unwrap();
        }
    }

    #[test]
    fn zero_redirect_policy_never_forwards_authorization_or_post_body() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let (url, received, worker) = server(format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            target.local_addr().unwrap()
        ));
        let response = agent()
            .post(&url)
            .set("Authorization", "Bearer secret")
            .send_json(serde_json::json!({"key":"secret"}))
            .unwrap();
        assert_eq!(response.status(), 307);
        received.recv().unwrap();
        worker.join().unwrap();
        assert!(matches!(target.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
        assert!(agent().0.config().proxy().is_none());
    }

    #[test]
    fn malformed_headers_fail_before_connect_and_body_reads_stay_bounded() {
        assert!(matches!(
            agent()
                .get("http://127.0.0.1:1")
                .set("Authorization", "a\r\nb")
                .call(),
            Err(Error::Transport(_))
        ));
        let response = Response::new(200, "OK", "123456789").unwrap();
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(4)
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"1234");
    }

    #[test]
    fn json_transcripts_do_not_acquire_a_new_implicit_size_limit() {
        let content = "x".repeat(10 * 1024 * 1024 + 1);
        let body = serde_json::to_string(&content).unwrap();
        let decoded: String = Response::new(200, "OK", &body)
            .unwrap()
            .into_json()
            .unwrap();
        assert_eq!(decoded, content);
    }
}
