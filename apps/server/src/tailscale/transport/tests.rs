use super::*;
use serde_json::json;
fn status() -> Value {
    json!({"Peer":{"nodekey:fixture":{"ID":"n-peer","DNSName":"worker.tail123.ts.net.","Online":true,"InNetworkMap":true,"TailscaleIPs":["100.64.0.2","fd7a:115c:a1e0::2"]}}})
}
#[test]
fn peer_is_an_exact_online_unexpired_netmap_node_with_only_tailnet_addresses() {
    let s = status();
    assert_eq!(
        peer(&s, "n-peer", "https://worker.tail123.ts.net")
            .unwrap()
            .ip,
        "100.64.0.2".parse::<IpAddr>().unwrap()
    );
    for (field, value) in [
        ("ID", json!("n-other")),
        ("DNSName", json!("other.tail123.ts.net.")),
        ("Online", json!(false)),
        ("InNetworkMap", json!(false)),
        ("Expired", json!(true)),
        ("TailscaleIPs", json!(["127.0.0.1"])),
        ("TailscaleIPs", json!(["100.64.0.2", "192.168.1.2"])),
        ("TailscaleIPs", json!(["8.8.8.8"])),
        ("TailscaleIPs", json!(["::1"])),
    ] {
        let mut s = status();
        s["Peer"]["nodekey:fixture"][field] = value;
        assert!(
            peer(&s, "n-peer", "https://worker.tail123.ts.net").is_err(),
            "{field}"
        );
    }
    let mut s = status();
    s["Peer"]["nodekey:duplicate"] = s["Peer"]["nodekey:fixture"].clone();
    assert!(peer(&s, "n-peer", "https://worker.tail123.ts.net").is_err());
    assert!(peer(&status(), "n-peer", "https://worker.tail123.ts.net/path").is_err());
}
#[cfg(unix)]
#[tokio::test]
async fn dial_self_redirect_or_wrong_upgrade_never_falls_back_to_a_system_connection() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for response in [
        "HTTP/1.1 200 OK\r\nDial-Self: true\r\nDial-Addr: 127.0.0.1:443\r\nContent-Length: 0\r\n\r\n",
        "HTTP/1.1 302 Found\r\nLocation: https://worker.tail123.ts.net\r\nContent-Length: 0\r\n\r\n",
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: wrong\r\nConnection: upgrade\r\n\r\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("socket");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut b = [0; 512];
                let n = stream.read(&mut b).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&b[..n]);
                if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
                assert!(bytes.len() < 4096);
            }
            let request = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
            assert!(request.starts_with("post /localapi/v0/dial http/1.1\r\n"));
            assert!(request.contains("dial-host: 100.64.0.2\r\n"));
            assert!(request.contains("dial-port: 443\r\n"));
            assert!(!request.contains("authorization"));
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let transport = Transport::new(
            socket,
            "n-self".into(),
            "https://controller.tail123.ts.net".into(),
            "127.0.0.1:8787".parse().unwrap(),
        )
        .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            transport.dial("100.64.0.2".parse().unwrap()),
        )
        .await
        .unwrap();
        assert_eq!(result.err(), Some("private_tailnet_dial_required"));
        server.await.unwrap();
    }
}
