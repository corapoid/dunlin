use super::*;
use serde_json::json;
fn expected() -> Expected {
    Expected {
        node: "n-fixture123".into(),
        origin: "https://worker.tail123.ts.net".into(),
        listen: "127.0.0.1:8787".parse().unwrap(),
    }
}
fn status() -> Value {
    json!({"Version":"1.92.0-fixture","BackendState":"Running","Self":{"ID":"n-fixture123","DNSName":"worker.tail123.ts.net.","Online":true}})
}
fn config() -> Value {
    json!({"TCP":{"443":{"HTTPS":true}},"Web":{"worker.tail123.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:8787"}}}}})
}
#[test]
fn identity_and_exact_background_https_proxy_are_required_not_just_a_dns_name() {
    let e = expected();
    assert!(check_status(&status(), &e).is_ok());
    assert!(check_serve(&config(), &e).is_ok());
    for (field, value) in [
        ("ID", json!("n-other")),
        ("DNSName", json!("worker.evil.ts.net.")),
        ("Expired", json!(true)),
        ("Online", json!(false)),
    ] {
        let mut s = status();
        s["Self"][field] = value;
        assert!(check_status(&s, &e).is_err(), "{field}");
    }
    let mut s = status();
    s["BackendState"] = json!("NeedsLogin");
    assert!(check_status(&s, &e).is_err());
    for proxy in [
        "http://localhost:8787",
        "http://192.168.1.20:8787",
        "https://127.0.0.1:8787",
        "http://127.0.0.1:8788",
        "http://127.0.0.1:8787/other",
    ] {
        let mut c = config();
        c["Web"]["worker.tail123.ts.net:443"]["Handlers"]["/"]["Proxy"] = json!(proxy);
        assert!(check_serve(&c, &e).is_err(), "{proxy}");
    }
    let mut c = config();
    c["Web"]["worker.tail123.ts.net:443"]["Handlers"]["/"]["Proxy"] =
        json!("http://127.0.0.1:8787/");
    assert!(check_serve(&c, &e).is_ok());
    let mut ipv6 = e.clone();
    ipv6.listen = "[::1]:8787".parse().unwrap();
    c["Web"]["worker.tail123.ts.net:443"]["Handlers"]["/"]["Proxy"] = json!("http://[::1]:8787");
    assert!(check_serve(&c, &ipv6).is_ok());
}
#[test]
fn funnel_foreground_overrides_and_unknown_exposure_modes_cannot_pass_inspection() {
    for patch in [
        json!({"AllowFunnel":{"other.tail123.ts.net:8443":true}}),
        json!({"AllowFunnel":{"worker.tail123.ts.net:443":"false"}}),
        json!({"Foreground":{"live":{"AllowFunnel":{"worker.tail123.ts.net:443":true}}}}),
        json!({"Services":{"svc:other":{}}}),
        json!({"FutureExposeMode":true}),
    ] {
        let mut c = config();
        c.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(check_serve(&c, &expected()).is_err());
    }
    let mut c = config();
    c["AllowFunnel"] = json!({"worker.tail123.ts.net:443":false});
    assert!(check_serve(&c, &expected()).is_ok());
    let mut c = config();
    c["Web"]["worker.tail123.ts.net:443"]["Handlers"]["/v1/worker"] =
        json!({"Proxy":"http://127.0.0.1:9999"});
    assert!(check_serve(&c, &expected()).is_err());
    for patch in [
        json!({"TCPForward":"127.0.0.1:8787"}),
        json!({"HTTP":true}),
        json!({"TerminateTLS":"worker.tail123.ts.net"}),
        json!({"ProxyProtocol":1}),
    ] {
        let mut c = config();
        c["TCP"]["443"]
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(check_serve(&c, &expected()).is_err());
    }
}
#[cfg(unix)]
async fn daemon(
    responses: Vec<Vec<u8>>,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tailscaled.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let handle = tokio::spawn(async move {
        for (index, response) in responses.into_iter().enumerate() {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut b = [0; 512];
                let n = stream.read(&mut b).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&b[..n]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
                assert!(request.len() < 4096);
            }
            let request = String::from_utf8(request).unwrap();
            let target = if index == 1 {
                "/localapi/v0/serve-config"
            } else {
                "/localapi/v0/status?peers=false"
            };
            assert!(request.starts_with(&format!("GET {target} HTTP/1.1\r\n")));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("host: local-tailscaled.sock\r\n")
            );
            assert!(!request.contains("Authorization"));
            // Oversize rejection may close before the entire response is written.
            let _ = stream.write_all(&response).await;
        }
    });
    (dir, path, handle)
}
fn http(value: Value, chunked: bool) -> Vec<u8> {
    let body = value.to_string();
    if chunked {
        format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",body.len()).into_bytes()
    } else {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }
}
#[cfg(unix)]
#[tokio::test]
async fn real_unix_http_inspection_is_read_only_handles_chunked_and_returns_no_remote_authority() {
    let (_dir, path, handle) = daemon(vec![
        http(status(), false),
        http(config(), true),
        http(status(), false),
    ])
    .await;
    let observation = inspect(&path, &expected()).await.unwrap();
    handle.await.unwrap();
    let public = serde_json::to_value(observation).unwrap();
    assert_eq!(public["application_authority"], false);
    assert_eq!(public["scope"], "local_serve_configuration_only");
    assert!(public.get("verified").is_none());
}
#[cfg(unix)]
#[tokio::test]
async fn daemon_errors_redirect_oversize_and_profile_switch_fail_without_followups_or_fallback() {
    let (_dir, path, handle) = daemon(vec![
        b"HTTP/1.1 302 Found\r\nLocation: https://evil.example/\r\nContent-Length: 0\r\n\r\n"
            .to_vec(),
    ])
    .await;
    assert_eq!(
        inspect(&path, &expected()).await.err(),
        Some("localapi_request_denied")
    );
    handle.await.unwrap();
    let oversized = json!({"padding":"x".repeat(MAX_BODY+1)});
    let (_dir, path, handle) = daemon(vec![http(oversized, false)]).await;
    assert_eq!(
        inspect(&path, &expected()).await.err(),
        Some("localapi_response_too_large")
    );
    handle.await.unwrap();
    let mut switched = status();
    switched["Self"]["ID"] = json!("n-new");
    let (_dir, path, handle) = daemon(vec![
        http(status(), false),
        http(config(), false),
        http(switched, false),
    ])
    .await;
    assert_eq!(
        inspect(&path, &expected()).await.err(),
        Some("tailscale_self_identity_mismatch")
    );
    handle.await.unwrap();
}
#[cfg(unix)]
#[tokio::test]
async fn socket_deadline_and_non_socket_paths_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let regular = dir.path().join("regular");
    std::fs::write(&regular, b"{}").unwrap();
    assert_eq!(
        inspect(&regular, &expected()).await.err(),
        Some("localapi_socket_required")
    );
    let path = dir.path().join("slow.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let slow = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    assert_eq!(
        inspect(&path, &expected()).await.err(),
        Some("localapi_deadline")
    );
    slow.abort();
}
