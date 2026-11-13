use super::*;
use crate::oauth::flow::tests::{callback, context, metadata, registration, tokens};
use crate::oauth::{Discovery, ResourceMetadata};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

struct Capture {
    path: String,
    headers: String,
    form: BTreeMap<String, String>,
}
struct Server {
    origin: String,
    worker: thread::JoinHandle<Vec<Capture>>,
}
impl Server {
    fn new(mut replies: Vec<(&'static str, u16, Value)>) -> Self {
        replies.insert(0, ("/token", 200, tokens()));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let mut captured = Vec::new();
            for (path, status, body) in replies {
                let started = Instant::now();
                let mut socket = loop {
                    match listener.accept() {
                        Ok((s, _)) => break s,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && started.elapsed() < Duration::from_secs(5) =>
                        {
                            thread::sleep(Duration::from_millis(5))
                        }
                        _ => panic!("OAuth request missing"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(&mut socket);
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    headers.push_str(&line);
                    assert!(headers.len() < 16384);
                    if line == "\r\n" {
                        break;
                    }
                }
                assert_eq!(
                    headers.lines().next().unwrap(),
                    format!("POST {path} HTTP/1.1")
                );
                assert!(!headers.to_ascii_lowercase().contains("cookie:"));
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (k, v) = line.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                assert!(length < 32768);
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                let form = url::form_urlencoded::parse(&bytes)
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                captured.push(Capture {
                    path: path.into(),
                    headers,
                    form,
                });
                if status == 0 {
                    continue;
                } // Request was received, response is lost.
                let body = body.to_string();
                write!(socket,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
            captured
        });
        Self { origin, worker }
    }
    fn finish(self) -> Vec<Capture> {
        self.worker.join().unwrap()
    }
}
fn open(
    replies: Vec<(&'static str, u16, Value)>,
    method: &str,
    revoke: bool,
) -> (Server, AuthorizationFlow, TokenSet) {
    let server = Server::new(replies);
    let origin = &server.origin;
    let discovery = Discovery::new(&format!("{origin}/mcp"), &[]).unwrap();
    let resource = ResourceMetadata {
        resource: format!("{origin}/mcp"),
        authorization_servers: vec![format!("{origin}/issuer")],
        scopes_supported: None,
    };
    let mut meta = metadata(origin, true);
    meta.client_id_metadata_document_supported = method == "cimd";
    if revoke {
        meta.revocation_endpoint = Some(format!("{origin}/revoke"));
        meta.revocation_endpoint_auth_methods_supported = Some(vec![
            "none".into(),
            "client_secret_basic".into(),
            "client_secret_post".into(),
        ]);
    }
    let flow = discovery
        .authorization_flow(&resource, meta, registration(origin, method))
        .unwrap();
    let request = flow.begin(context(), vec!["files:read".into()]).unwrap();
    let token = flow
        .finish(
            &context(),
            &callback(&flow, &request, Some(token_issuer(&flow))),
        )
        .unwrap();
    (server, flow, token)
}
fn token_issuer(flow: &AuthorizationFlow) -> &str {
    &flow.issuer
}
fn refreshed(access: &str, refresh: Option<&str>) -> Value {
    let mut value =
        json!({"access_token":access,"token_type":"Bearer","expires_in":3600,"scope":"files:read"});
    if let Some(refresh) = refresh {
        value["refresh_token"] = json!(refresh);
    }
    value
}

#[test]
fn rotating_and_non_rotating_refresh_replace_access_and_preserve_the_correct_refresh_token() {
    for method in ["public", "basic", "post", "cimd"] {
        let (server, flow, mut token) = open(
            vec![
                ("/token", 200, refreshed("access-two", Some("refresh-two"))),
                ("/token", 200, refreshed("access-three", None)),
            ],
            method,
            true,
        );
        token.expires = Instant::now() - Duration::from_secs(1);
        flow.refresh(&context(), &mut token).unwrap();
        assert_eq!(
            token
                .access_token(&context(), &format!("{}/mcp", server.origin))
                .unwrap(),
            "access-two"
        );
        flow.refresh(&context(), &mut token).unwrap();
        assert_eq!(
            token
                .access_token(&context(), &format!("{}/mcp", server.origin))
                .unwrap(),
            "access-three"
        );
        assert_eq!(
            token.refresh_token.as_ref().unwrap().0.as_str(),
            "refresh-two"
        );
        let captures = server.finish();
        assert_eq!(captures.len(), 3);
        assert_eq!(captures[1].form["grant_type"], "refresh_token");
        assert_eq!(captures[1].form["refresh_token"], "refresh-fixture");
        assert_eq!(captures[2].form["refresh_token"], "refresh-two");
        assert_eq!(captures[1].form["scope"], "files:read");
        assert!(captures[1].form["resource"].ends_with("/mcp"));
        assert!(!captures[1].form.contains_key("code"));
        for capture in &captures[1..] {
            if method == "basic" {
                assert!(
                    capture
                        .headers
                        .to_ascii_lowercase()
                        .contains("authorization: basic ")
                );
                assert!(!capture.form.contains_key("client_secret"));
            } else {
                assert!(
                    !capture
                        .headers
                        .to_ascii_lowercase()
                        .contains("authorization:")
                );
                assert_eq!(
                    capture.form["client_id"],
                    if method == "cimd" {
                        crate::oauth::flow::tests::CIMD_ID
                    } else {
                        "client:+/?"
                    }
                );
                if method == "post" {
                    assert_eq!(capture.form["client_secret"], "secret: +/?");
                }
            }
        }
    }
}

#[test]
fn lost_refresh_reply_blocks_positive_reuse_but_allows_explicit_revocation() {
    let (server, flow, mut token) = open(
        vec![
            ("/token", 0, Value::Null),
            ("/revoke", 200, json!({})),
            ("/revoke", 200, json!({})),
        ],
        "public",
        true,
    );
    assert!(matches!(
        flow.refresh(&context(), &mut token),
        Err(Error::Transport)
    ));
    assert_eq!(token.status(), CredentialStatus::RefreshUncertain);
    assert!(
        token
            .access_token(&context(), &format!("{}/mcp", server.origin))
            .is_err()
    );
    assert!(matches!(
        flow.refresh(&context(), &mut token),
        Err(Error::Unauthorized)
    ));
    flow.revoke(&context(), &mut token).unwrap();
    assert_eq!(token.status(), CredentialStatus::Revoked);
    assert!(!token.has_refresh_token());
    assert!(token.access_token.0.is_empty());
    let captures = server.finish();
    assert_eq!(captures.len(), 4);
    assert_eq!(captures[2].form["token_type_hint"], "refresh_token");
    assert_eq!(captures[3].form["token_type_hint"], "access_token");
}

#[test]
fn failed_remote_revocation_keeps_local_access_disabled_and_explicit_retry_is_idempotent() {
    let (server, flow, mut token) = open(
        vec![
            ("/revoke", 503, json!({})),
            ("/revoke", 200, json!({})),
            ("/revoke", 200, json!({})),
        ],
        "basic",
        true,
    );
    assert!(matches!(
        flow.revoke(&context(), &mut token),
        Err(Error::Http(503))
    ));
    assert_eq!(token.status(), CredentialStatus::RevocationUnconfirmed);
    assert!(
        token
            .access_token(&context(), &format!("{}/mcp", server.origin))
            .is_err()
    );
    assert!(matches!(
        flow.refresh(&context(), &mut token),
        Err(Error::Unauthorized)
    ));
    flow.revoke(&context(), &mut token).unwrap();
    flow.revoke(&context(), &mut token).unwrap();
    assert_eq!(token.status(), CredentialStatus::Revoked);
    let captures = server.finish();
    assert_eq!(captures.len(), 4);
    assert_eq!(captures[1].form["token"], captures[2].form["token"]);
    assert_eq!(captures[3].form["token"], "access-fixture");
    assert!(captures.iter().skip(1).all(|c| c.path == "/revoke"));
}

#[test]
fn foreign_context_revision_or_registration_cannot_refresh_or_disable_credentials() {
    let (server, mut flow, mut token) = open(vec![], "public", true);
    for field in ["user", "workspace", "connection", "revision"] {
        let mut wrong = context();
        match field {
            "user" => wrong.user_id = "other".into(),
            "workspace" => wrong.workspace_id = "other".into(),
            "connection" => wrong.connection_id = "other".into(),
            _ => wrong.connection_revision += 1,
        }
        assert!(matches!(
            flow.refresh(&wrong, &mut token),
            Err(Error::Unauthorized)
        ));
        assert!(matches!(
            flow.revoke(&wrong, &mut token),
            Err(Error::Unauthorized)
        ));
        assert_eq!(token.status(), CredentialStatus::Active);
    }
    flow.registration.client_id = "different-client".into();
    assert!(matches!(
        flow.refresh(&context(), &mut token),
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        flow.revoke(&context(), &mut token),
        Err(Error::Unauthorized)
    ));
    assert_eq!(token.status(), CredentialStatus::Active);
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn absent_revocation_support_is_reported_without_claiming_remote_success() {
    let (server, flow, mut token) = open(vec![], "public", false);
    assert!(matches!(
        flow.revoke(&context(), &mut token),
        Err(Error::Unsupported)
    ));
    assert_eq!(token.status(), CredentialStatus::RevocationUnconfirmed);
    assert!(
        token
            .access_token(&context(), &format!("{}/mcp", server.origin))
            .is_err()
    );
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn refresh_failure_or_scope_escalation_never_restores_an_active_credential() {
    for (status, body) in [
        (400, json!({"error":"invalid_grant"})),
        (
            200,
            json!({"access_token":"expanded","token_type":"Bearer","expires_in":3600,"scope":"admin:write","refresh_token":"rotated"}),
        ),
    ] {
        let (server, flow, mut token) = open(vec![("/token", status, body)], "post", true);
        assert!(flow.refresh(&context(), &mut token).is_err());
        assert_eq!(token.status(), CredentialStatus::RefreshUncertain);
        assert!(
            token
                .access_token(&context(), &format!("{}/mcp", server.origin))
                .is_err()
        );
        assert!(flow.refresh(&context(), &mut token).is_err());
        assert_eq!(server.finish().len(), 2);
    }
}

#[test]
fn access_only_credentials_can_be_revoked_and_refresh_absence_is_not_an_attempt() {
    let (server, flow, mut token) = open(
        vec![(
            "/revoke",
            200,
            json!({"ignored":"body is not a protocol result"}),
        )],
        "post",
        true,
    );
    token.refresh_token = None;
    assert!(matches!(
        flow.refresh(&context(), &mut token),
        Err(Error::Unsupported)
    ));
    assert_eq!(token.status(), CredentialStatus::Active);
    flow.revoke(&context(), &mut token).unwrap();
    assert_eq!(token.status(), CredentialStatus::Revoked);
    let captures = server.finish();
    assert_eq!(captures.len(), 2);
    assert_eq!(captures[1].form["token"], "access-fixture");
    assert_eq!(captures[1].form["token_type_hint"], "access_token");
    assert!(!captures[1].form.contains_key("resource"));
}

#[test]
fn revocation_metadata_obeys_origin_policy_reserved_queries_and_authentication_defaults() {
    let origin = "http://127.0.0.1:9";
    let discovery = Discovery::new(&format!("{origin}/mcp"), &[]).unwrap();
    let resource = ResourceMetadata {
        resource: format!("{origin}/mcp"),
        authorization_servers: vec![format!("{origin}/issuer")],
        scopes_supported: None,
    };
    for url in [
        "https://unapproved.example/revoke".to_owned(),
        format!("{origin}/revoke?token=untrusted"),
    ] {
        let mut meta = metadata(origin, true);
        meta.revocation_endpoint = Some(url);
        assert!(
            discovery
                .authorization_flow(&resource, meta, registration(origin, "public"))
                .is_err()
        );
    }
    for method in ["public", "basic"] {
        let mut meta = metadata(origin, true);
        meta.revocation_endpoint = Some(format!("{origin}/revoke"));
        let flow = discovery
            .authorization_flow(&resource, meta, registration(origin, method))
            .unwrap();
        assert_eq!(flow.revocation_endpoint.is_some(), method == "basic");
    }
}

mod persistence_tests;
