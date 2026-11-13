use super::*;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    thread,
};

const REDIRECT: &str = "http://127.0.0.1:3333/callback";
pub(super) const CIMD_ID: &str = "https://client.example/oauth/client.json";
pub(super) fn context() -> AuthContext {
    AuthContext {
        user_id: "user".into(),
        workspace_id: "workspace".into(),
        connection_id: "connection".into(),
        connection_revision: 1,
    }
}
pub(super) fn metadata(origin: &str, required: bool) -> AuthorizationMetadata {
    serde_json::from_value(json!({"issuer":format!("{origin}/issuer"),"authorization_endpoint":format!("{origin}/authorize"),"token_endpoint":format!("{origin}/token"),"response_types_supported":["code"],"code_challenge_methods_supported":["S256"],"authorization_response_iss_parameter_supported":required,"token_endpoint_auth_methods_supported":["none","client_secret_basic","client_secret_post"]})).unwrap()
}
pub(super) fn registration(origin: &str, method: &str) -> Registration {
    let issuer = format!("{origin}/issuer");
    match method {
        "cimd" => Registration::client_metadata_document(&issuer, CIMD_ID, REDIRECT).unwrap(),
        "basic" => {
            Registration::confidential_basic(&issuer, "client:+/?", REDIRECT, "secret: +/?".into())
                .unwrap()
        }
        "post" => {
            Registration::confidential_post(&issuer, "client:+/?", REDIRECT, "secret: +/?".into())
                .unwrap()
        }
        _ => Registration::public(&issuer, "client:+/?", REDIRECT).unwrap(),
    }
}
fn build(origin: &str, method: &str, required: bool) -> AuthorizationFlow {
    let discovery = Discovery::new(&format!("{origin}/mcp"), &[]).unwrap();
    let resource = ResourceMetadata {
        resource: format!("{origin}/mcp"),
        authorization_servers: vec![format!("{origin}/issuer")],
        scopes_supported: None,
    };
    let mut metadata = metadata(origin, required);
    metadata.client_id_metadata_document_supported = method == "cimd";
    discovery
        .authorization_flow(&resource, metadata, registration(origin, method))
        .unwrap()
}
pub(super) fn parameters(url: &str) -> BTreeMap<String, String> {
    Url::parse(url)
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}
pub(super) fn callback(
    flow: &AuthorizationFlow,
    request: &AuthorizationRequest,
    issuer: Option<&str>,
) -> String {
    let mut url = Url::parse(REDIRECT).unwrap();
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("state", &parameters(&request.url)["state"])
            .append_pair("code", "code +/&?");
        if let Some(issuer) = issuer {
            query.append_pair("iss", issuer);
        }
    }
    assert_eq!(flow.registration.redirect_uri, REDIRECT);
    url.into()
}
struct Capture {
    headers: String,
    form: BTreeMap<String, String>,
}
struct Server {
    origin: String,
    worker: thread::JoinHandle<Capture>,
}
impl Server {
    fn new(status: u16, body: Value) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let started = Instant::now();
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && started.elapsed() < Duration::from_secs(5) =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    _ => panic!("token request missing"),
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
            assert!(headers.starts_with("POST /token HTTP/1.1"));
            assert!(!headers.to_lowercase().contains("cookie:"));
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            assert!(length < 32768);
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            let form = url::form_urlencoded::parse(&bytes)
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            let body = body.to_string();
            write!(socket,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            Capture { headers, form }
        });
        Self { origin, worker }
    }
    fn finish(self) -> Capture {
        self.worker.join().unwrap()
    }
}
pub(super) fn tokens() -> Value {
    json!({"access_token":"access-fixture","refresh_token":"refresh-fixture","token_type":"Bearer","expires_in":3600,"scope":"files:read"})
}

#[test]
fn real_code_exchange_preserves_pkce_resource_redirect_and_each_client_authentication_method() {
    for method in ["public", "basic", "post", "cimd"] {
        let server = Server::new(200, tokens());
        let flow = build(&server.origin, method, true);
        let ctx = context();
        let request = flow
            .begin(ctx.clone(), vec!["files:read".into(), "files:write".into()])
            .unwrap();
        let params = parameters(&request.url);
        assert_eq!(params["resource"], format!("{}/mcp", server.origin));
        assert_eq!(params["code_challenge_method"], "S256");
        assert!(!params.contains_key("code_verifier"));
        assert!(!params.contains_key("client_secret"));
        assert_eq!(
            params["client_id"],
            if method == "cimd" {
                CIMD_ID
            } else {
                "client:+/?"
            }
        );
        let callback = callback(&flow, &request, Some(&flow.issuer));
        let mut wrong = ctx.clone();
        wrong.user_id = "another-user".into();
        assert!(matches!(
            flow.finish(&wrong, &callback),
            Err(Error::Unauthorized)
        ));
        let token = flow.finish(&ctx, &callback).unwrap();
        assert_eq!(
            token
                .access_token(&ctx, &format!("{}/mcp", server.origin))
                .unwrap(),
            "access-fixture"
        );
        assert_eq!(token.scopes(), ["files:read"]);
        assert!(token.has_refresh_token());
        assert_eq!(token.issuer(), flow.issuer);
        assert!(matches!(
            flow.finish(&ctx, &callback),
            Err(Error::Unauthorized)
        ));
        assert!(
            token
                .access_token(&wrong, &format!("{}/mcp", server.origin))
                .is_err()
        );
        assert!(
            token
                .access_token(&ctx, &format!("{}/other", server.origin))
                .is_err()
        );
        let captured = server.finish();
        assert_eq!(captured.form["grant_type"], "authorization_code");
        assert_eq!(captured.form["code"], "code +/&?");
        assert_eq!(captured.form["redirect_uri"], REDIRECT);
        assert_eq!(captured.form["resource"], params["resource"]);
        assert_eq!(
            URL_SAFE_NO_PAD.encode(Sha256::digest(captured.form["code_verifier"].as_bytes())),
            params["code_challenge"]
        );
        let auth = captured.headers.lines().find_map(|line| {
            line.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                .map(|(_, v)| v.trim())
        });
        if method == "basic" {
            let encoded = auth.unwrap().strip_prefix("Basic ").unwrap();
            assert_eq!(
                String::from_utf8(STANDARD.decode(encoded).unwrap()).unwrap(),
                "client%3A%2B%2F%3F:secret%3A+%2B%2F%3F"
            );
            assert!(!captured.form.contains_key("client_secret"));
        } else {
            assert!(auth.is_none());
            assert_eq!(captured.form["client_id"], params["client_id"]);
            if method == "post" {
                assert_eq!(captured.form["client_secret"], "secret: +/?");
            } else {
                assert!(!captured.form.contains_key("client_secret"));
            }
        }
    }
}

#[test]
fn cimds_require_explicit_as_support_and_do_not_inherit_preregistered_credentials() {
    let origin = "http://127.0.0.1:9";
    let discovery = Discovery::new(&format!("{origin}/mcp"), &[]).unwrap();
    let resource = ResourceMetadata {
        resource: format!("{origin}/mcp"),
        authorization_servers: vec![format!("{origin}/issuer")],
        scopes_supported: None,
    };
    assert!(matches!(
        discovery.authorization_flow(
            &resource,
            metadata(origin, true),
            registration(origin, "cimd")
        ),
        Err(Error::Unsupported)
    ));
    let mut no_pkce = metadata(origin, true);
    no_pkce.client_id_metadata_document_supported = true;
    no_pkce.code_challenge_methods_supported.clear();
    assert!(matches!(
        discovery.authorization_flow(&resource, no_pkce, registration(origin, "cimd")),
        Err(Error::Unsupported)
    ));
    let mut no_public = metadata(origin, true);
    no_public.client_id_metadata_document_supported = true;
    no_public.token_endpoint_auth_methods_supported = Some(vec!["client_secret_basic".into()]);
    assert!(matches!(
        discovery.authorization_flow(&resource, no_public, registration(origin, "cimd")),
        Err(Error::Unsupported)
    ));
    let mut wrong_issuer = metadata(origin, true);
    wrong_issuer.client_id_metadata_document_supported = true;
    wrong_issuer.issuer.push_str("/other");
    assert!(matches!(
        discovery.authorization_flow(&resource, wrong_issuer, registration(origin, "cimd")),
        Err(Error::Protocol)
    ));
    let cimd = build(origin, "cimd", true);
    let preregistered = discovery
        .authorization_flow(
            &resource,
            metadata(origin, true),
            Registration::public(&format!("{origin}/issuer"), CIMD_ID, REDIRECT).unwrap(),
        )
        .unwrap();
    assert_ne!(
        cimd.credential_binding(),
        preregistered.credential_binding()
    );
    assert_eq!(
        cimd.credential_binding(),
        build(origin, "cimd", true).credential_binding()
    );
}

#[test]
fn issuer_validation_precedes_errors_and_rejects_missing_or_mismatched_iss_without_network() {
    for (required, iss, error) in [
        (true, None, false),
        (true, Some("http://127.0.0.1:9/issuer/"), false),
        (false, Some("http://127.0.0.1:9/issuer/"), false),
        (true, Some("http://evil.invalid"), true),
    ] {
        let flow = build("http://127.0.0.1:9", "public", required);
        let request = flow.begin(context(), vec![]).unwrap();
        let mut uri = Url::parse(&callback(&flow, &request, iss)).unwrap();
        if error {
            uri.query_pairs_mut()
                .append_pair("error", "access_denied")
                .append_pair("error_description", "never display this");
        }
        assert!(matches!(
            flow.finish(&context(), uri.as_str()),
            Err(Error::Protocol)
        ));
        assert!(matches!(
            flow.finish(&context(), uri.as_str()),
            Err(Error::Unauthorized)
        ));
    }
    let flow = build("http://127.0.0.1:9", "public", true);
    let request = flow.begin(context(), vec![]).unwrap();
    let mut uri = Url::parse(&callback(&flow, &request, Some(&flow.issuer))).unwrap();
    uri.query_pairs_mut().append_pair("error", "access_denied");
    assert!(matches!(
        flow.finish(&context(), uri.as_str()),
        Err(Error::Unauthorized)
    ));
}

#[test]
fn optional_issuer_can_be_absent_and_token_expiry_is_enforced_locally() {
    let server = Server::new(200, tokens());
    let flow = build(&server.origin, "public", false);
    let request = flow.begin(context(), vec!["files:read".into()]).unwrap();
    let mut token = flow
        .finish(&context(), &callback(&flow, &request, None))
        .unwrap();
    token.expires = Instant::now() - Duration::from_secs(1);
    assert!(
        token
            .access_token(&context(), &format!("{}/mcp", server.origin))
            .is_err()
    );
    server.finish();
}

#[test]
fn callback_shape_and_authenticated_context_cannot_change_or_consume_another_users_record() {
    let flow = build("http://127.0.0.1:9", "public", true);
    let request = flow.begin(context(), vec![]).unwrap();
    let uri = callback(&flow, &request, Some(&flow.issuer));
    let mut duplicate = Url::parse(&uri).unwrap();
    duplicate
        .query_pairs_mut()
        .append_pair("state", &parameters(&request.url)["state"]);
    assert!(matches!(
        flow.finish(&context(), duplicate.as_str()),
        Err(Error::Protocol)
    ));
    let mut wrong = context();
    wrong.workspace_id = "other-workspace".into();
    assert!(matches!(
        flow.finish(&wrong, &uri),
        Err(Error::Unauthorized)
    ));
    wrong = context();
    wrong.connection_id = "another-connection".into();
    assert!(matches!(
        flow.finish(&wrong, &uri),
        Err(Error::Unauthorized)
    ));
    wrong = context();
    wrong.connection_revision += 1;
    assert!(matches!(
        flow.finish(&wrong, &uri),
        Err(Error::Unauthorized)
    ));
    assert_eq!(flow.pending.lock().unwrap().len(), 1);
    assert!(matches!(
        flow.finish(&context(), &uri.replace("/callback?", "/wrong?")),
        Err(Error::Protocol)
    ));
}

#[test]
fn expired_state_is_rejected_and_pending_admission_has_a_hard_bound() {
    let flow = build("http://127.0.0.1:9", "public", true);
    let mut states = HashMap::new();
    for n in 0..MAX_PENDING {
        let mut ctx = context();
        ctx.connection_id = format!("connection-{n}");
        let request = flow.begin(ctx, vec![]).unwrap();
        assert!(
            states
                .insert(parameters(&request.url)["state"].clone(), request)
                .is_none()
        );
    }
    assert!(matches!(flow.begin(context(), vec![]), Err(Error::Limit)));
    for record in flow.pending.lock().unwrap().values_mut() {
        record.created = Instant::now() - Duration::from_secs(STATE_SECONDS + 1);
    }
    let request = flow.begin(context(), vec![]).unwrap();
    assert_eq!(flow.pending.lock().unwrap().len(), 1);
    flow.pending
        .lock()
        .unwrap()
        .get_mut(&hash(&parameters(&request.url)["state"]))
        .unwrap()
        .created = Instant::now() - Duration::from_secs(STATE_SECONDS + 1);
    assert!(matches!(
        flow.finish(&context(), &callback(&flow, &request, Some(&flow.issuer))),
        Err(Error::Unauthorized)
    ));
}

#[test]
fn registration_and_endpoint_pins_reject_issuer_changes_and_reserved_query_injection() {
    let origin = "http://127.0.0.1:9";
    let discovery = Discovery::new(&format!("{origin}/mcp"), &[]).unwrap();
    let resource = ResourceMetadata {
        resource: format!("{origin}/mcp"),
        authorization_servers: vec![format!("{origin}/issuer")],
        scopes_supported: None,
    };
    let wrong_registration =
        Registration::public(&format!("{origin}/another"), "client", REDIRECT).unwrap();
    assert!(matches!(
        discovery.authorization_flow(&resource, metadata(origin, true), wrong_registration),
        Err(Error::Protocol)
    ));
    let mut meta = metadata(origin, true);
    meta.authorization_endpoint
        .push_str("?redirect_uri=https://evil.invalid");
    assert!(matches!(
        discovery.authorization_flow(&resource, meta, registration(origin, "public")),
        Err(Error::Protocol)
    ));
}

#[test]
fn token_http_failure_is_not_retried_and_scope_escalation_is_rejected() {
    let server = Server::new(
        400,
        json!({"error":"invalid_grant","error_description":"provider secret"}),
    );
    let flow = build(&server.origin, "public", true);
    let request = flow.begin(context(), vec!["files:read".into()]).unwrap();
    let uri = callback(&flow, &request, Some(&flow.issuer));
    assert!(matches!(
        flow.finish(&context(), &uri),
        Err(Error::Http(400))
    ));
    assert!(matches!(
        flow.finish(&context(), &uri),
        Err(Error::Unauthorized)
    ));
    server.finish();
    for field in ["scope", "token_type", "expires_in"] {
        let mut body = tokens();
        body[field] = match field {
            "scope" => json!("admin:write"),
            "token_type" => json!("MAC"),
            _ => json!(0),
        };
        let server = Server::new(200, body);
        let flow = build(&server.origin, "public", true);
        let request = flow.begin(context(), vec!["files:read".into()]).unwrap();
        assert!(matches!(
            flow.finish(&context(), &callback(&flow, &request, Some(&flow.issuer))),
            Err(Error::Protocol)
        ));
        server.finish();
    }
}
