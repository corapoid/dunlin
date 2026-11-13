//! Read-only LocalAPI inspection. A local configuration observation is neither
//! remote attestation nor application authority. No Serve/Funnel writes exist.
use serde::Serialize;
use serde_json::Value;
use std::{
    net::SocketAddr,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

mod transport;
pub(crate) use transport::Transport;

const MAX_BODY: usize = 1024 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug)]
struct Expected {
    node: String,
    origin: String,
    listen: SocketAddr,
}
impl Expected {
    fn validate(&self) -> Result<&str, &'static str> {
        let definition = dunlin_domain::hosts::HostDefinition {
            name: "Local Serve inspection".into(),
            node_id: self.node.clone(),
            serve_origin: self.origin.clone(),
            capability_policy: dunlin_domain::hosts::HostCapabilityPolicy {
                max_parallel: 1,
                harnesses: vec![],
            },
        };
        definition.validate().map_err(|_| "invalid_expected_host")?;
        if !self.listen.ip().is_loopback() || self.listen.port() == 0 {
            return Err("loopback_backend_required");
        }
        self.origin
            .strip_prefix("https://")
            .ok_or("invalid_expected_host")
    }
}
/// Not Deserialize: only successful reads of the operator-selected local socket
/// construct this observation. It is deliberately not a remote-host proof.
#[derive(Serialize)]
struct LocalServeObservation {
    scope: &'static str,
    node_id: String,
    serve_origin: String,
    loopback_backend: String,
    observed_at: u64,
    daemon_version: String,
    application_authority: bool,
}
fn object<'a>(
    value: &'a Value,
    code: &'static str,
) -> Result<&'a serde_json::Map<String, Value>, &'static str> {
    value.as_object().ok_or(code)
}
fn known(value: &serde_json::Map<String, Value>, keys: &[&str]) -> Result<(), &'static str> {
    if value.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err("unsupported_serve_config");
    }
    Ok(())
}
fn absent_or_empty(value: Option<&Value>) -> bool {
    value.is_none_or(|value| value.as_object().is_some_and(|map| map.is_empty()))
}
fn check_status(status: &Value, expected: &Expected) -> Result<String, &'static str> {
    let hostname = expected.validate()?;
    if status["BackendState"] != "Running" {
        return Err("tailscale_not_running");
    }
    let peer = &status["Self"];
    if peer["ID"].as_str() != Some(expected.node.as_str())
        || peer["DNSName"]
            .as_str()
            .map(|name| name.strip_suffix('.').unwrap_or(name))
            != Some(hostname)
        || peer["Online"] != true
        || peer.get("Expired").is_some_and(|expired| expired != false)
    {
        return Err("tailscale_self_identity_mismatch");
    }
    let version = status["Version"]
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= 128 && !v.chars().any(char::is_control))
        .ok_or("tailscale_version_missing")?;
    Ok(version.to_owned())
}
fn check_serve(config: &Value, expected: &Expected) -> Result<(), &'static str> {
    let hostname = expected.validate()?;
    let config = object(config, "serve_config_missing")?;
    // New modes require deliberate review. Do not silently ignore future
    // exposure fields, ephemeral foreground overlays or virtual services.
    known(
        config,
        &["TCP", "Web", "AllowFunnel", "Foreground", "Services"],
    )?;
    if !absent_or_empty(config.get("Foreground")) || !absent_or_empty(config.get("Services")) {
        return Err("unsupported_serve_overlays");
    }
    if let Some(funnel) = config.get("AllowFunnel")
        && object(funnel, "invalid_funnel_config")?
            .values()
            .any(|allowed| allowed != false)
    {
        return Err("funnel_enabled");
    }
    let tcp = object(
        config.get("TCP").ok_or("serve_https_missing")?,
        "serve_https_missing",
    )?;
    let port = object(
        tcp.get("443").ok_or("serve_https_missing")?,
        "serve_https_missing",
    )?;
    known(
        port,
        &[
            "HTTPS",
            "HTTP",
            "TCPForward",
            "TerminateTLS",
            "ProxyProtocol",
        ],
    )?;
    if port.get("HTTPS") != Some(&Value::Bool(true))
        || port.get("HTTP").is_some_and(|v| v != false)
        || port.get("TCPForward").is_some_and(|v| v != "")
        || port.get("TerminateTLS").is_some_and(|v| v != "")
        || port.get("ProxyProtocol").is_some_and(|v| v != 0)
    {
        return Err("unsupported_serve_tcp_mode");
    }
    let web = object(
        config.get("Web").ok_or("serve_proxy_missing")?,
        "serve_proxy_missing",
    )?;
    let server = object(
        web.get(&format!("{hostname}:443"))
            .ok_or("serve_proxy_missing")?,
        "serve_proxy_missing",
    )?;
    known(server, &["Handlers"])?;
    let handlers = object(
        server.get("Handlers").ok_or("serve_proxy_missing")?,
        "serve_proxy_missing",
    )?;
    if handlers.len() != 1 {
        return Err("unsupported_serve_path_overrides");
    }
    let handler = object(
        handlers.get("/").ok_or("serve_proxy_missing")?,
        "serve_proxy_missing",
    )?;
    known(
        handler,
        &["Proxy", "Path", "Text", "Redirect", "AcceptAppCaps"],
    )?;
    if handler.get("Path").is_some_and(|v| v != "")
        || handler.get("Text").is_some_and(|v| v != "")
        || handler.get("Redirect").is_some_and(|v| v != "")
    {
        return Err("unsupported_serve_handler");
    }
    // The CLI commonly normalizes http proxy targets with a trailing slash.
    let target = format!("http://{}", expected.listen);
    if !handler
        .get("Proxy")
        .and_then(Value::as_str)
        .is_some_and(|proxy| proxy == target || proxy == format!("{target}/"))
    {
        return Err("serve_backend_mismatch");
    }
    Ok(())
}

#[cfg(unix)]
async fn get_json(socket: &Path, path: &'static str) -> Result<Value, &'static str> {
    use axum::body::Bytes;
    use http_body_util::{BodyExt, Full};
    use hyper_util::rt::TokioIo;
    let stream = tokio::net::UnixStream::connect(socket)
        .await
        .map_err(|_| "localapi_unavailable")?;
    // The explicitly configured socket is a local account trust boundary. Do
    // not accept a replaced socket served by another unprivileged OS user.
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
    struct ConnectionGuard(tokio::task::JoinHandle<Result<(), hyper::Error>>);
    impl Drop for ConnectionGuard {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _connection = ConnectionGuard(tokio::spawn(connection));
    let request = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "local-tailscaled.sock")
        .header("Connection", "close")
        .header("Accept", "application/json")
        .body(Full::new(Bytes::new()))
        .map_err(|_| "localapi_protocol_error")?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|_| "localapi_protocol_error")?;
    if response.status() != hyper::StatusCode::OK {
        return Err("localapi_request_denied");
    }
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| "localapi_protocol_error")?;
        if let Some(data) = frame.data_ref() {
            if data.len() > MAX_BODY.saturating_sub(bytes.len()) {
                return Err("localapi_response_too_large");
            }
            bytes.extend_from_slice(data);
        }
    }
    serde_json::from_slice(&bytes).map_err(|_| "invalid_localapi_json")
}
#[cfg(unix)]
async fn inspect(
    socket: &Path,
    expected: &Expected,
) -> Result<LocalServeObservation, &'static str> {
    use std::os::unix::fs::FileTypeExt;
    expected.validate()?;
    if !socket.is_absolute()
        || !std::fs::symlink_metadata(socket)
            .map_err(|_| "localapi_socket_missing")?
            .file_type()
            .is_socket()
    {
        return Err("localapi_socket_required");
    }
    tokio::time::timeout(PROBE_TIMEOUT, async {
        let before = get_json(socket, "/localapi/v0/status?peers=false").await?;
        let version = check_status(&before, expected)?;
        let config = get_json(socket, "/localapi/v0/serve-config").await?;
        check_serve(&config, expected)?;
        // Catch an identity/profile switch while the two observations are read.
        let after = get_json(socket, "/localapi/v0/status?peers=false").await?;
        if check_status(&after, expected)? != version {
            return Err("tailscale_changed_during_inspection");
        }
        Ok(LocalServeObservation {
            scope: "local_serve_configuration_only",
            node_id: expected.node.clone(),
            serve_origin: expected.origin.clone(),
            loopback_backend: expected.listen.to_string(),
            observed_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "invalid_clock")?
                .as_secs(),
            daemon_version: version,
            application_authority: false,
        })
    })
    .await
    .map_err(|_| "localapi_deadline")?
}

pub(crate) fn command(args: &[String]) -> Result<(), String> {
    const USAGE: &str = "host inspect-local --socket ABSOLUTE_PATH --node-id STABLE_ID --origin https://node.tail.ts.net --listen 127.0.0.1:8787";
    if args.first().map(String::as_str) != Some("inspect-local") || args.len() != 9 {
        return Err(USAGE.into());
    }
    let mut values = std::collections::BTreeMap::new();
    for pair in args[1..].as_chunks::<2>().0 {
        if !["--socket", "--node-id", "--origin", "--listen"].contains(&pair[0].as_str())
            || values.insert(pair[0].as_str(), pair[1].as_str()).is_some()
        {
            return Err(USAGE.into());
        }
    }
    let expected = Expected {
        node: values["--node-id"].into(),
        origin: values["--origin"].into(),
        listen: values["--listen"]
            .parse()
            .map_err(|_| "invalid_loopback_backend")?,
    };
    expected.validate().map_err(str::to_owned)?;
    #[cfg(unix)]
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "inspection_runtime_unavailable")?;
        let result = runtime
            .block_on(inspect(Path::new(values["--socket"]), &expected))
            .map_err(str::to_owned)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&result).map_err(|_| "inspection_encoding_failed")?
        );
        Ok(())
    }
    #[cfg(not(unix))]
    {
        Err("LocalAPI Unix socket inspection is unavailable on this platform".into())
    }
}
#[cfg(test)]
mod tests;
