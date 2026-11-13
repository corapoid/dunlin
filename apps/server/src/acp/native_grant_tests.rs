//! Native bridge authority must remain pinned across revoke/regrant ABA.
use super::*;
use dunlin_coordination::service::{Service, request};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message as ClientMessage, client::IntoClientRequest},
};
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
const PEER: &str = r#"
import json, os, select, sys
pid = os.getpid()
log = 'input-' + str(pid)
release = 'release-' + str(pid)
emitted = 'emitted-' + str(pid)
with open('bridge-pids', 'a') as f: f.write(str(pid) + '\n')
buffer = b''
while True:
    if os.path.exists(release) and not os.path.exists(emitted):
        print(json.dumps({'jsonrpc':'2.0','method':'fixture/private','params':{'text':'stale stdout must not be forwarded'}}), flush=True)
        with open(emitted, 'w') as f: f.write('emitted')
    ready, _, _ = select.select([sys.stdin.fileno()], [], [], .01)
    if not ready: continue
    data = os.read(sys.stdin.fileno(), 65536)
    if not data: break
    buffer += data
    while b'\n' in buffer:
        raw, buffer = buffer.split(b'\n', 1)
        if not raw: continue
        value = json.loads(raw)
        with open(log, 'a') as f: f.write(json.dumps(value) + '\n')
        print(json.dumps({'jsonrpc':'2.0','id':value.get('id'),'result':{'pid':pid,'echo':value.get('params',{})}}), flush=True)
"#;
struct Fixture {
    _dir: tempfile::TempDir,
    state: AppState,
    user: User,
    device: Device,
    token: String,
    primary: Project,
    additional: Project,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let primary_path = dir.path().join("primary");
        let additional_path = dir.path().join("additional");
        std::fs::create_dir(&primary_path).unwrap();
        std::fs::create_dir(&additional_path).unwrap();
        let peer = dir.path().join("peer.py");
        std::fs::write(&peer, PEER).unwrap();
        let store = crate::store::Store::open_in_memory().unwrap();
        let user = store.create_user("native-bridge", "member").unwrap();
        let (device, token) = store.create_device(&user.id, "native-device").unwrap();
        let primary = store
            .create_project("primary", primary_path.to_str().unwrap())
            .unwrap();
        let additional = store
            .create_project("additional", additional_path.to_str().unwrap())
            .unwrap();
        store.grant(&primary.id, &user.id).unwrap();
        store.grant(&additional.id, &user.id).unwrap();
        let mut state = AppState::new(
            store,
            crate::config::Config {
                db: dir.path().join("relay.db"),
                sandbox: false,
                harness: format!("python3 -u {}", peer.display()),
                ..Default::default()
            },
        );
        state.coordination = Some(std::sync::Arc::new(
            Service::start(&dir.path().join("coordination")).unwrap(),
        ));
        Self {
            _dir: dir,
            state,
            user,
            device,
            token,
            primary,
            additional,
        }
    }
    fn tracker(&self) -> Tracker {
        let projects = vec![self.primary.clone(), self.additional.clone()];
        let (native_grants, candidates) = {
            let store = self.state.store.lock().unwrap();
            (
                projects
                    .iter()
                    .map(|p| store.capture_native_grant(&p.id, &self.user.id).unwrap())
                    .collect(),
                projects
                    .iter()
                    .map(|p| {
                        store
                            .native_checkout_candidate(
                                &p.id,
                                &self.user.id,
                                MetadataPermission::Write,
                            )
                            .unwrap()
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let validated = candidates
            .into_iter()
            .map(|c| c.validate_filesystem().unwrap())
            .collect::<Vec<_>>();
        let store = self.state.store.lock().unwrap();
        let native_checkouts = validated
            .into_iter()
            .map(|v| store.authorize_native_checkout(v).unwrap())
            .collect::<Vec<_>>();
        Tracker {
            native_grants,
            native_projects: projects,
            allowed_roots: native_checkouts
                .iter()
                .map(|a| a.root().to_owned())
                .collect(),
            native_checkouts,
            allowed_project_ids: vec![self.additional.id.clone()],
            ..Default::default()
        }
    }
    fn aba(&self, project: &Project) {
        let store = self.state.store.lock().unwrap();
        store.revoke_grant(&project.id, &self.user.id).unwrap();
        store.grant(&project.id, &self.user.id).unwrap();
        assert!(
            store
                .project_for_user(&project.id, &self.user.id)
                .unwrap()
                .is_some()
        );
    }
}
struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn server(f: &Fixture) -> (String, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "ws://{}/acp?project={}&roots={}",
        listener.local_addr().unwrap(),
        f.primary.id,
        f.additional.id
    );
    let router = crate::api::router(f.state.clone());
    let job = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (url, Server(job))
}
async fn connect(url: &str, token: &str) -> Socket {
    let mut req = url.into_client_request().unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    connect_async(req).await.unwrap().0
}
async fn rpc(socket: &mut Socket, id: u64, marker: &str) -> Value {
    socket
        .send(ClientMessage::Text(
            json!({"jsonrpc":"2.0","id":id,"method":"fixture/probe","params":{"marker":marker}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(frame) = socket.next().await {
            if let ClientMessage::Text(text) = frame.unwrap() {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["id"] == id {
                    return value;
                }
            }
        }
        panic!("Bridge closed before probe result");
    })
    .await
    .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_primary_and_additional_epoch_aba_stop_input_stdout_and_old_process() {
    for extra in [false, true] {
        let f = Fixture::new();
        let (url, _server) = server(&f).await;
        let mut socket = connect(&url, &f.token).await;
        let first = rpc(&mut socket, 1, "before ABA").await;
        let pid = first["result"]["pid"].as_i64().unwrap() as i32;
        let root = std::path::Path::new(&f.primary.path);
        let revoked = if extra { &f.additional } else { &f.primary };
        {
            let store = f.state.store.lock().unwrap();
            // Block forwarding's Store check while the real peer emits stdout,
            // then revoke and restore under this same cut. No inactive interval
            // is visible to the bridge, only the changed epoch.
            std::fs::write(root.join(format!("release-{pid}")), "release").unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !root.join(format!("emitted-{pid}")).exists() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "Peer did not emit stdout"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            store.revoke_grant(&revoked.id, &f.user.id).unwrap();
            store.grant(&revoked.id, &f.user.id).unwrap();
        }
        let _=socket.send(ClientMessage::Text(json!({"jsonrpc":"2.0","id":2,"method":"fixture/probe","params":{"marker":"stale input must not be forwarded"}}).to_string().into())).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(frame) = socket.next().await {
                match frame {
                    Ok(ClientMessage::Text(text)) => {
                        assert!(!text.contains("stale stdout"));
                        assert!(!text.contains("stale input"));
                    }
                    Ok(ClientMessage::Close(_)) | Err(_) => break,
                    _ => {}
                }
            }
        })
        .await
        .expect("Old bridge did not close after native grant ABA");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while unsafe { libc::kill(pid, 0) == 0 } {
            assert!(
                tokio::time::Instant::now() < deadline,
                "Old bridge process survived"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            !std::fs::read_to_string(root.join(format!("input-{pid}")))
                .unwrap()
                .contains("stale input")
        );
        let mut fresh = connect(&url, &f.token).await;
        let reply = rpc(&mut fresh, 3, "fresh explicit connection").await;
        assert_eq!(
            reply["result"]["echo"]["marker"],
            "fresh explicit connection"
        );
        assert_ne!(reply["result"]["pid"], pid);
        fresh.close(None).await.unwrap();
    }
}
#[test]
fn tracker_and_new_broker_tokens_cannot_refresh_primary_or_additional_epochs_implicitly() {
    for extra in [false, true] {
        let f = Fixture::new();
        let mut tracker = f.tracker();
        let before = tracker.authority_guard(&f.state, &f.user, &f.device, &f.primary);
        assert!(before());
        let service = f.state.coordination.as_ref().unwrap();
        let token = service
            .grant_guarded(
                dunlin_coordination::Scope {
                    authority: f.user.id.clone(),
                    project: f.primary.id.clone(),
                },
                "session".into(),
                f.primary.path.clone(),
                Some(before.clone()),
            )
            .unwrap();
        assert!(request(service.socket(), &token.token, "agents_list", json!({})).is_ok());
        f.aba(if extra { &f.additional } else { &f.primary });
        assert!(!tracker.roots_authorized(&f.state, &f.user));
        assert!(!before());
        let (close, _) = mpsc::channel(1);
        let rejected=tracker.on_client_message(&f.state,&f.user,&f.device,&f.primary,"old-connection",&close,&json!({"jsonrpc":"2.0","id":1,"method":"session/new","params":{"cwd":f.primary.path,"mcpServers":[]}})).unwrap_err();
        assert_eq!(rejected["error"]["code"], -32003);
        assert!(request(service.socket(), &token.token, "agents_list", json!({})).is_err());
        // A freshly minted token from the old tracker still pins the old epoch.
        let replacement = service
            .grant_guarded(
                dunlin_coordination::Scope {
                    authority: f.user.id.clone(),
                    project: f.primary.id.clone(),
                },
                "replacement".into(),
                f.primary.path.clone(),
                Some(tracker.authority_guard(&f.state, &f.user, &f.device, &f.primary)),
            )
            .unwrap();
        assert!(
            request(
                service.socket(),
                &replacement.token,
                "agents_list",
                json!({})
            )
            .is_err()
        );
        let fresh = f.tracker();
        assert!(fresh.roots_authorized(&f.state, &f.user));
        assert!(fresh
            .authority_guard(&f.state, &f.user, &f.device, &f.primary)(
        ));
    }
}
