//! TLS streams only through tailscaled's private dial upgrade. No system DNS,
//! environment proxy, direct TCP, Dial-Self fallback or TLS verification bypass.
use super::*;
use axum::body::Bytes;
use http_body_util::Full;
use hyper_util::rt::TokioIo;
use std::{net::IpAddr, path::PathBuf, sync::Arc};
use tokio::io::{AsyncRead, AsyncWrite};

pub(crate) trait PeerIo: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> PeerIo for T {}
pub(crate) type PeerStream = Box<dyn PeerIo>;
#[derive(Clone)]
pub(crate) struct Transport {
    socket: PathBuf,
    expected: Expected,
    tls: Arc<rustls::ClientConfig>,
}
#[derive(Clone, PartialEq, Eq, Debug)]
struct Peer {
    hostname: String,
    ip: IpAddr,
}
fn private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => (u32::from(ip) & 0xffc00000) == 0x64400000,
        IpAddr::V6(ip) => ip.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}
fn peer(status: &Value, node: &str, origin: &str) -> Result<Peer, &'static str> {
    let definition = dunlin_domain::hosts::HostDefinition {
        name: "Peer".into(),
        node_id: node.into(),
        serve_origin: origin.into(),
        capability_policy: dunlin_domain::hosts::HostCapabilityPolicy {
            max_parallel: 1,
            harnesses: vec![],
        },
    };
    definition.validate().map_err(|_| "invalid_peer_identity")?;
    let hostname = origin
        .strip_prefix("https://")
        .ok_or("invalid_peer_identity")?;
    let peers = object(&status["Peer"], "tailscale_peer_missing")?;
    let mut matches = peers
        .values()
        .filter(|peer| peer["ID"].as_str() == Some(node));
    let found = matches.next().ok_or("tailscale_peer_missing")?;
    if matches.next().is_some()
        || found["DNSName"]
            .as_str()
            .map(|n| n.strip_suffix('.').unwrap_or(n))
            != Some(hostname)
        || found["Online"] != true
        || found["InNetworkMap"] != true
        || found.get("Expired").is_some_and(|v| v != false)
    {
        return Err("tailscale_peer_identity_mismatch");
    }
    let addresses = found["TailscaleIPs"]
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 8)
        .ok_or("tailscale_peer_address_missing")?;
    let mut parsed = Vec::new();
    for address in addresses {
        let ip = address
            .as_str()
            .ok_or("invalid_tailnet_address")?
            .parse::<IpAddr>()
            .map_err(|_| "invalid_tailnet_address")?;
        if !private_ip(ip) {
            return Err("non_tailnet_peer_address");
        }
        parsed.push(ip);
    }
    parsed.sort();
    parsed.dedup();
    Ok(Peer {
        hostname: hostname.into(),
        ip: parsed[0],
    })
}
impl Transport {
    pub(crate) fn new(
        socket: PathBuf,
        node_id: String,
        serve_origin: String,
        listen: SocketAddr,
    ) -> Result<Self, &'static str> {
        let expected = Expected {
            node: node_id,
            origin: serve_origin,
            listen,
        };
        expected.validate()?;
        if !socket.is_absolute() {
            return Err("localapi_socket_required");
        }
        let roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| "tls_configuration_failed")?
        .with_root_certificates(roots)
        .with_no_client_auth();
        Ok(Self {
            socket,
            expected,
            tls: Arc::new(config),
        })
    }
    #[cfg(test)]
    pub(crate) fn with_test_roots(mut self, roots: rustls::RootCertStore) -> Self {
        self.tls = Arc::new(
            rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
        );
        self
    }
    #[cfg(unix)]
    async fn observe_peer(&self, node: &str, origin: &str) -> Result<Peer, &'static str> {
        if node == self.expected.node {
            return Err("remote_peer_required");
        }
        tokio::time::timeout(PROBE_TIMEOUT, async {
            inspect(&self.socket, &self.expected).await?;
            let status = get_json(&self.socket, "/localapi/v0/status").await?;
            check_status(&status, &self.expected)?;
            peer(&status, node, origin)
        })
        .await
        .map_err(|_| "peer_inspection_deadline")?
    }
    #[cfg(unix)]
    pub(crate) async fn verify_peer(&self, node: &str, origin: &str) -> Result<(), &'static str> {
        self.observe_peer(node, origin).await.map(|_| ())
    }
    #[cfg(unix)]
    pub(crate) async fn connect(
        &self,
        node: &str,
        origin: &str,
    ) -> Result<PeerStream, &'static str> {
        tokio::time::timeout(Duration::from_secs(5), async {
            let pinned = self.observe_peer(node, origin).await?;
            let stream = self.dial(pinned.ip).await?;
            let name = rustls::pki_types::ServerName::try_from(pinned.hostname.clone())
                .map_err(|_| "invalid_peer_identity")?;
            let stream = tokio_rustls::TlsConnector::from(self.tls.clone())
                .connect(name, stream)
                .await
                .map_err(|_| "peer_tls_verification_failed")?;
            if self.observe_peer(node, origin).await? != pinned {
                return Err("peer_changed_during_connection");
            }
            Ok(Box::new(stream) as PeerStream)
        })
        .await
        .map_err(|_| "peer_connection_deadline")?
    }
    #[cfg(unix)]
    async fn dial(&self, ip: IpAddr) -> Result<PeerStream, &'static str> {
        if !private_ip(ip) {
            return Err("non_tailnet_peer_address");
        }
        let stream = tokio::net::UnixStream::connect(&self.socket)
            .await
            .map_err(|_| "localapi_unavailable")?;
        let uid = stream
            .peer_cred()
            .map_err(|_| "localapi_peer_unavailable")?
            .uid();
        if uid != 0 && uid != unsafe { libc::geteuid() } {
            return Err("localapi_peer_denied");
        }
        let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
            .max_buf_size(16 * 1024)
            .max_headers(64)
            .handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .map_err(|_| "localapi_protocol_error")?;
        struct Guard(tokio::task::JoinHandle<Result<(), hyper::Error>>);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _connection = Guard(tokio::spawn(connection.with_upgrades()));
        let request = hyper::Request::builder()
            .method("POST")
            .uri("/localapi/v0/dial")
            .header("Host", "local-tailscaled.sock")
            .header("Upgrade", "ts-dial")
            .header("Connection", "upgrade")
            .header("Dial-Host", ip.to_string())
            .header("Dial-Port", "443")
            .header("Dial-Network", "tcp")
            .body(Full::new(Bytes::new()))
            .map_err(|_| "localapi_protocol_error")?;
        let mut response = sender
            .send_request(request)
            .await
            .map_err(|_| "localapi_protocol_error")?;
        // In particular HTTP200 Dial-Self is never honored: it can mean a LAN
        // or public destination which this application's transport forbids.
        if response.status() != hyper::StatusCode::SWITCHING_PROTOCOLS
            || response.headers().get_all("Upgrade").iter().count() != 1
            || !response
                .headers()
                .get("Upgrade")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.eq_ignore_ascii_case("ts-dial"))
        {
            return Err("private_tailnet_dial_required");
        }
        let upgraded = hyper::upgrade::on(&mut response)
            .await
            .map_err(|_| "localapi_upgrade_failed")?;
        Ok(Box::new(TokioIo::new(upgraded)) as PeerStream)
    }
    #[cfg(not(unix))]
    pub(crate) async fn verify_peer(&self, _node: &str, _origin: &str) -> Result<(), &'static str> {
        Err("localapi_transport_unsupported")
    }
    #[cfg(not(unix))]
    pub(crate) async fn connect(
        &self,
        _node: &str,
        _origin: &str,
    ) -> Result<PeerStream, &'static str> {
        Err("localapi_transport_unsupported")
    }
}
#[cfg(test)]
mod tests;
