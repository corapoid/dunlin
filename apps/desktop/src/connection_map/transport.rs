//! Bounded presence HTTP, separate from credentials and native UI.
use dunlin_domain::presence::Snapshot;
use std::{collections::HashSet, io::Read, time::Duration};

pub(crate) fn heartbeat(
    agent: &dunlin_http::Agent,
    url: &str,
    token: &str,
) -> Result<Snapshot, String> {
    // The caller validates the relay URL and supplies the shared no-proxy,
    // no-redirect agent so credentials cannot follow a redirect to another host.
    let response = agent
        .post(&format!("{}/v1/presence", url.trim_end_matches('/')))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(4))
        .call()
        .map_err(|error| match error {
            dunlin_http::Error::Status(404 | 405, _) => {
                "Update relay to show connected users".to_string()
            }
            dunlin_http::Error::Status(401 | 403, _) => {
                "Access denied · check server authorization".to_string()
            }
            _ => "Connection unknown · reconnecting…".to_string(),
        })?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read presence")?;
    if bytes.len() > 256 * 1024 {
        return Err("Presence response is too large".into());
    }
    decode(&bytes)
}

fn decode(bytes: &[u8]) -> Result<Snapshot, String> {
    let invalid = || "Invalid presence response · update relay".to_string();
    let snapshot: Snapshot = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let mut identities = HashSet::new();
    if snapshot.users.len() > 512
        || snapshot.self_user_id.is_empty()
        || snapshot.ttl_secs == 0
        || snapshot.workload.as_ref().is_some_and(|workload| {
            workload.active > 1 || workload.queued > 512 || workload.interrupted > 512
        })
        || snapshot.users.iter().any(|user| {
            user.id.is_empty()
                || user.id.len() > 256
                || user.name.len() > 1024
                || user.devices == 0
                || !identities.insert(&user.id)
        })
        || !snapshot
            .users
            .iter()
            .any(|user| user.id == snapshot.self_user_id)
    {
        return Err(invalid());
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn body() -> serde_json::Value {
        json!({"self_user_id":"me", "ttl_secs":30, "users":[{"id":"me", "name":"Me", "devices":1}]})
    }

    fn server(
        status: &str,
        extra_headers: &str,
        body: String,
    ) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let status = status.to_string();
        let extra_headers = extra_headers.to_string();
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let bytes = socket.read(&mut buffer).unwrap();
                assert_ne!(bytes, 0);
                request.extend_from_slice(&buffer[..bytes]);
                assert!(request.len() < 16 * 1024);
            }
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes());
            String::from_utf8(request).unwrap()
        });
        (url, handle)
    }

    fn agent() -> dunlin_http::Agent {
        dunlin_http::AgentBuilder::new()
            .try_proxy_from_env(false)
            .redirects(0)
            .build()
    }

    #[test]
    fn sends_authenticated_heartbeat_and_decodes_actual_http_response() {
        let (url, handle) = server("200 OK", "", body().to_string());
        let snapshot = heartbeat(&agent(), &url, "fixture-token").unwrap();
        assert_eq!(snapshot.self_user_id, "me");
        let request = handle.join().unwrap();
        assert!(request.starts_with("POST /v1/presence HTTP/1.1\r\n"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer fixture-token")
        );
    }

    #[test]
    fn unsupported_and_unauthorized_relays_are_not_empty_success() {
        for (status, expected) in [
            ("404 Not Found", "Update relay"),
            ("401 Unauthorized", "Access denied"),
            ("403 Forbidden", "Access denied"),
        ] {
            let (url, handle) = server(
                status,
                "",
                "{\"error\":\"do not echo remote details\"}".into(),
            );
            let error = heartbeat(&agent(), &url, "secret-fixture-token").unwrap_err();
            assert!(error.starts_with(expected));
            assert!(!error.contains("secret-fixture-token"));
            assert!(!error.contains("remote details"));
            handle.join().unwrap();
        }
    }

    #[test]
    fn redirect_is_not_followed_with_the_bearer_token() {
        let (url, handle) = server(
            "307 Temporary Redirect",
            "Location: https://example.invalid/token-theft\r\n",
            String::new(),
        );
        assert!(heartbeat(&agent(), &url, "fixture-token").is_err());
        handle.join().unwrap();
    }

    #[test]
    fn oversized_response_is_rejected() {
        let (url, handle) = server("200 OK", "", " ".repeat(256 * 1024 + 1));
        assert_eq!(
            heartbeat(&agent(), &url, "fixture-token").unwrap_err(),
            "Presence response is too large"
        );
        handle.join().unwrap();
    }

    #[test]
    fn malformed_ambiguous_and_unbounded_rosters_are_rejected() {
        assert!(decode(b"not json").is_err());
        for invalid in [
            json!({"self_user_id":"missing", "ttl_secs":30, "users":[]}),
            json!({"self_user_id":"me", "ttl_secs":30, "users":[{"id":"me","name":"Me","devices":0}]}),
            json!({"self_user_id":"me", "ttl_secs":30, "users":[{"id":"me","name":"Me","devices":1},{"id":"me","name":"Imposter","devices":1}]}),
        ] {
            assert!(decode(invalid.to_string().as_bytes()).is_err());
        }
        let mut invalid = body();
        invalid["users"][0]["name"] = json!("x".repeat(1025));
        assert!(decode(invalid.to_string().as_bytes()).is_err());
    }
}
