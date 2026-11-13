use super::*;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

struct Server {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    thread: thread::JoinHandle<()>,
}
impl Server {
    fn new(build: impl FnOnce(&str) -> Vec<(&'static str, u16, Value)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let replies = build(&origin);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let thread = thread::spawn(move || {
            for (path, status, value) in replies {
                let started = Instant::now();
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && started.elapsed() < Duration::from_secs(5) =>
                        {
                            thread::sleep(Duration::from_millis(5))
                        }
                        _ => panic!("missing metadata request"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(&mut socket);
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    request.push_str(&line);
                    assert!(request.len() < 8192);
                    if line == "\r\n" {
                        break;
                    }
                }
                let expected = if path.starts_with("POST ") {
                    path.to_owned()
                } else {
                    format!("GET {path}")
                };
                assert_eq!(
                    request.lines().next().unwrap(),
                    format!("{expected} HTTP/1.1")
                );
                if path.starts_with("POST ") {
                    let length = request
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    assert!(length < 8192);
                    let mut bytes = vec![0; length];
                    reader.read_exact(&mut bytes).unwrap();
                    let body: Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(body["method"], "server/discover");
                    assert_eq!(
                        body["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
                        crate::metadata::VERSION
                    );
                }
                assert!(!request.to_ascii_lowercase().contains("authorization:"));
                assert!(!request.to_ascii_lowercase().contains("cookie:"));
                captured.lock().unwrap().push(request);
                let body = value.to_string();
                let headers = value
                    .get("__headers")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .map(|v| format!("{}\r\n", v.as_str().unwrap()))
                            .collect::<String>()
                    })
                    .unwrap_or_default();
                write!(socket,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",body.len()).unwrap();
            }
        });
        Self {
            origin,
            requests,
            thread,
        }
    }
    fn finish(self) -> Vec<String> {
        self.thread.join().unwrap();
        Arc::try_unwrap(self.requests)
            .ok()
            .unwrap()
            .into_inner()
            .unwrap()
    }
}
fn resource(origin: &str, issuer: &str) -> Value {
    json!({"resource":format!("{origin}/mcp"),"authorization_servers":[issuer],"scopes_supported":["files:read"]})
}
fn authorization(origin: &str) -> Value {
    json!({"issuer":format!("{origin}/tenant"),"authorization_endpoint":format!("{origin}/authorize"),"token_endpoint":format!("{origin}/token"),"response_types_supported":["code"],"code_challenge_methods_supported":["S256"],"authorization_response_iss_parameter_supported":true})
}

#[test]
fn challenge_lexer_handles_other_schemes_quoted_commas_escapes_and_bws() {
    let challenge=Challenge::parse(&[r#"Basic realm="other", Bearer resource_metadata="https://mcp.example/meta?x=a,b", scope = "files:read files:write", error_description="say \"no\", then retry""#]).unwrap().unwrap();
    assert_eq!(
        challenge.resource_metadata.as_deref(),
        Some("https://mcp.example/meta?x=a,b")
    );
    assert_eq!(challenge.scopes.unwrap(), ["files:read", "files:write"]);
    assert!(
        Challenge::parse(&["Basic realm=example"])
            .unwrap()
            .is_none()
    );
    for value in [
        "Bearer scope=read, scope=write",
        "Bearer, Bearer",
        "Bearer scope=\"unfinished",
        "Bearer scope=\"read  write\"",
        "Bearer\r\nInjected: yes",
    ] {
        assert!(Challenge::parse(&[value]).is_err(), "{value}");
    }
}

#[test]
fn well_known_fallback_obeys_resource_and_oauth_oidc_priority_without_credentials() {
    let server = Server::new(|origin| {
        vec![
            ("/.well-known/oauth-protected-resource/mcp", 404, json!({})),
            (
                "/.well-known/oauth-protected-resource",
                200,
                resource(origin, &format!("{origin}/tenant")),
            ),
            (
                "/.well-known/oauth-authorization-server/tenant",
                404,
                json!({}),
            ),
            ("/.well-known/openid-configuration/tenant", 404, json!({})),
            (
                "/tenant/.well-known/openid-configuration",
                200,
                authorization(origin),
            ),
        ]
    });
    let discovery = Discovery::new(&format!("{}/mcp", server.origin), &[]).unwrap();
    let protected = discovery.protected_resource(None).unwrap();
    let metadata = discovery.authorization_server(&protected, 0).unwrap();
    assert!(metadata.authorization_response_iss_parameter_supported);
    assert_eq!(server.finish().len(), 5);
}

#[test]
fn challenge_location_wins_and_challenged_scopes_need_not_be_in_supported_scopes() {
    let server = Server::new(|origin| {
        vec![
            (
                "/custom-meta",
                200,
                resource(origin, &format!("{origin}/tenant")),
            ),
            (
                "/.well-known/oauth-authorization-server/tenant",
                200,
                authorization(origin),
            ),
        ]
    });
    let challenge = Challenge::parse(&[&format!(
        "Bearer resource_metadata=\"{}/custom-meta\", scope=\"admin:write\"",
        server.origin
    )])
    .unwrap()
    .unwrap();
    let discovery = Discovery::new(&format!("{}/mcp", server.origin), &[]).unwrap();
    let protected = discovery.protected_resource(Some(&challenge)).unwrap();
    assert_eq!(
        protected.requested_scopes(Some(&challenge)),
        vec!["admin:write"]
    );
    assert_eq!(protected.requested_scopes(None), vec!["files:read"]);
    discovery.authorization_server(&protected, 0).unwrap();
    assert_eq!(server.finish().len(), 2);
}

#[test]
fn metadata_cannot_expand_network_origins_or_substitute_resource_and_issuer() {
    let server = Server::new(|origin| {
        vec![(
            "/.well-known/oauth-protected-resource/mcp",
            200,
            resource(origin, "https://unapproved.example/tenant"),
        )]
    });
    let discovery = Discovery::new(&format!("{}/mcp", server.origin), &[]).unwrap();
    let protected = discovery.protected_resource(None).unwrap();
    assert!(matches!(
        discovery.authorization_server(&protected, 0),
        Err(Error::Unsupported)
    ));
    assert_eq!(server.finish().len(), 1);
    let server = Server::new(|origin| {
        vec![(
            "/.well-known/oauth-protected-resource/mcp",
            200,
            json!({"resource":format!("{origin}/mcp-evil"),"authorization_servers":[format!("{origin}/tenant")]}),
        )]
    });
    let discovery = Discovery::new(&format!("{}/mcp", server.origin), &[]).unwrap();
    assert!(matches!(
        discovery.protected_resource(None),
        Err(Error::Protocol)
    ));
    assert_eq!(server.finish().len(), 1);
    let server = Server::new(|origin| {
        let mut metadata = authorization(origin);
        metadata["issuer"] = json!(format!("{origin}/tenant/"));
        vec![
            (
                "/.well-known/oauth-protected-resource/mcp",
                200,
                resource(origin, &format!("{origin}/tenant")),
            ),
            (
                "/.well-known/oauth-authorization-server/tenant",
                200,
                metadata,
            ),
        ]
    });
    let discovery = Discovery::new(&format!("{}/mcp", server.origin), &[]).unwrap();
    let protected = discovery.protected_resource(None).unwrap();
    assert!(matches!(
        discovery.authorization_server(&protected, 0),
        Err(Error::Protocol)
    ));
    assert_eq!(server.finish().len(), 2);
}

#[test]
fn explicitly_allowed_separate_authorization_origin_is_supported() {
    let auth = Server::new(|origin| {
        vec![(
            "/.well-known/oauth-authorization-server/tenant",
            200,
            authorization(origin),
        )]
    });
    let server = Server::new(|origin| {
        vec![(
            "/.well-known/oauth-protected-resource/mcp",
            200,
            resource(origin, &format!("{}/tenant", auth.origin)),
        )]
    });
    let discovery = Discovery::new(
        &format!("{}/mcp", server.origin),
        std::slice::from_ref(&auth.origin),
    )
    .unwrap();
    let protected = discovery.protected_resource(None).unwrap();
    let metadata = discovery.authorization_server(&protected, 0).unwrap();
    assert_eq!(metadata.issuer, format!("{}/tenant", auth.origin));
    assert_eq!(server.finish().len(), 1);
    assert_eq!(auth.finish().len(), 1);
}

#[test]
fn invalid_success_and_http_errors_are_not_hidden_by_discovery_fallback() {
    for status in [200, 401, 403, 302, 500] {
        let server = Server::new(|_| {
            vec![(
                "/.well-known/oauth-protected-resource/mcp",
                status,
                json!({}),
            )]
        });
        let discovery = Discovery::new(&format!("{}/mcp", server.origin), &[]).unwrap();
        assert!(discovery.protected_resource(None).is_err());
        assert_eq!(server.finish().len(), 1);
    }
    let server = Server::new(|origin| {
        let mut metadata = authorization(origin);
        metadata["code_challenge_methods_supported"] = json!(["plain"]);
        vec![
            (
                "/.well-known/oauth-protected-resource/mcp",
                200,
                resource(origin, &format!("{origin}/tenant")),
            ),
            (
                "/.well-known/oauth-authorization-server/tenant",
                200,
                metadata,
            ),
        ]
    });
    let discovery = Discovery::new(&format!("{}/mcp", server.origin), &[]).unwrap();
    let protected = discovery.protected_resource(None).unwrap();
    assert!(matches!(
        discovery.authorization_server(&protected, 0),
        Err(Error::Unsupported)
    ));
    assert_eq!(server.finish().len(), 2);
}

#[test]
fn actual_unauthenticated_probe_uses_all_challenge_headers_and_the_advertised_location() {
    let server = Server::new(|origin| {
        vec![
            (
                "POST /mcp",
                401,
                json!({"__headers":["WWW-Authenticate: Basic realm=other",format!("WWW-Authenticate: Bearer resource_metadata=\"{origin}/custom-meta\", scope=\"files:read\"")]}),
            ),
            (
                "/custom-meta",
                200,
                resource(origin, &format!("{origin}/tenant")),
            ),
        ]
    });
    let discovery = Discovery::new(&format!("{}/mcp", server.origin), &[]).unwrap();
    assert_eq!(
        discovery
            .protected_resource_from_server()
            .unwrap()
            .metadata
            .resource,
        format!("{}/mcp", server.origin)
    );
    assert_eq!(server.finish().len(), 2);
}

#[test]
fn well_known_construction_preserves_paths_and_root_issuer_priority() {
    let urls = issuer_urls(&Url::parse("https://auth.example").unwrap());
    assert_eq!(
        urls.iter().map(Url::as_str).collect::<Vec<_>>(),
        vec![
            "https://auth.example/.well-known/oauth-authorization-server",
            "https://auth.example/.well-known/openid-configuration"
        ]
    );
    let urls = issuer_urls(&Url::parse("https://auth.example/tenant%2Fchild").unwrap());
    assert_eq!(
        urls[0].path(),
        "/.well-known/oauth-authorization-server/tenant%2Fchild"
    );
    assert_eq!(
        urls[2].path(),
        "/tenant%2Fchild/.well-known/openid-configuration"
    );
}
