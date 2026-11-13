use super::*;
mod history;
use crate::{config::Config, store::Store};
use ::ahp as sdk;
use dunlin_session_runtime::{Lease, Session, TurnPhase};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message as WsMessage, client::IntoClientRequest},
};
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
struct Fixture {
    state: AppState,
    cookie: String,
    user: String,
    device: String,
    workspace: String,
    project: String,
    session: Session,
    lease: Lease,
    dir: tempfile::TempDir,
}
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("relay.db")).unwrap();
    let user = store.create_user("ahp-owner", "admin").unwrap();
    let (device, _) = store.create_device(&user.id, "browser").unwrap();
    let token = store.create_browser_session(&device.id, 3600).unwrap();
    let workspace = store.create_workspace(&user, "AHP").unwrap();
    let project = store
        .create_team_project(&user.id, &workspace.id, "project")
        .unwrap();
    let a = store
        .team_project_context(&workspace.id, &project.id, &user.id)
        .unwrap();
    let j = store.interactive_journal();
    let session = j.create(&a, &workspace.id, &project.id).unwrap();
    let lease = j.acquire(&a, &session.id, 0, "fixture-worker").unwrap();
    drop(j);
    let state = AppState::new(
        store,
        Config {
            experimental_interactive_runtime: true,
            sandbox: false,
            db: dir.path().join("relay.db"),
            ..Default::default()
        },
    );
    Fixture {
        state,
        cookie: format!("dunlin_web_session={token}"),
        user: user.id,
        device: device.id,
        workspace: workspace.id,
        project: project.id,
        session,
        lease,
        dir,
    }
}
struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn server(state: &AppState) -> (String, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = crate::api::router(state.clone());
    let job = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (origin, Server(job))
}
async fn connect(origin: &str, cookie: &str) -> Socket {
    let mut request = format!("{}/v1/web/ahp", origin.replace("http://", "ws://"))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", origin.parse().unwrap());
    request
        .headers_mut()
        .insert("cookie", cookie.parse().unwrap());
    connect_async(request).await.unwrap().0
}
async fn next(socket: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                WsMessage::Text(t) => return serde_json::from_str(&t).unwrap(),
                WsMessage::Ping(b) => socket.send(WsMessage::Pong(b)).await.unwrap(),
                v => panic!("Unexpected frame {v:?}"),
            }
        }
    })
    .await
    .unwrap()
}
async fn rpc(socket: &mut Socket, id: u64, method: &str, params: Value) -> Value {
    socket
        .send(WsMessage::Text(
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    loop {
        let v = next(socket).await;
        if v["id"] == id {
            return v;
        }
    }
}
fn produce(f: &Fixture, text: &str) -> String {
    let store = f.state.store.lock().unwrap();
    let a = store
        .team_project_context(&f.workspace, &f.project, &f.user)
        .unwrap();
    let j = store.interactive_journal();
    let t = j
        .enqueue(
            &a,
            &f.session.id,
            &uuid::Uuid::now_v7().to_string(),
            "private user prompt",
        )
        .unwrap();
    j.claim_next(&a, &f.session.id, &f.lease).unwrap();
    j.transition(&a, &f.session.id, &f.lease, &t, TurnPhase::Running)
        .unwrap();
    j.output(&a, &f.session.id, &f.lease, &t, text).unwrap();
    j.transition_timed(
        &a,
        &f.session.id,
        &f.lease,
        &t,
        TurnPhase::Completed,
        Some(10),
    )
    .unwrap();
    t
}
#[tokio::test]
async fn two_real_websocket_observers_replay_and_restart_snapshot_without_polling() {
    let f = fixture();
    let (origin, old_server) = server(&f.state).await;
    let chat = format!("ahp-chat:/{}", f.session.id);
    let session = format!("ahp-session:/{}", f.session.id);
    let mut first = connect(&origin, &f.cookie).await;
    let mut second = connect(&origin, &f.cookie).await;
    for (socket, id) in [(&mut first, "one"), (&mut second, "two")] {
        let initialized=rpc(socket,1,"initialize",json!({"channel":ROOT,"clientId":id,"protocolVersions":["0.9.0","1.0.0"],"initialSubscriptions":[ROOT,chat,session]})).await;
        let result: ahp_types::commands::InitializeResult =
            serde_json::from_value(initialized["result"].clone()).unwrap();
        assert_eq!(result.protocol_version, "1.0.0");
        assert_eq!(result.snapshots.len(), 3);
    }
    let listed = rpc(
        &mut first,
        2,
        "listSessions",
        json!({"channel":ROOT,"limit":1}),
    )
    .await;
    assert_eq!(listed["result"]["items"][0]["resource"], session);
    let baseline = {
        f.state
            .store
            .lock()
            .unwrap()
            .interactive_journal()
            .channel_clock()
            .unwrap()
            .sequence
    };
    second.close(None).await.unwrap();
    produce(&f, "streamed while other observer disconnected");
    let mut first_seq = baseline;
    let mut got_text = false;
    let mut got_summary = false;
    let mut got_session = false;
    while !(got_text && got_summary && got_session) {
        let event = next(&mut first).await;
        if event["method"] == "action" {
            let envelope: ahp_types::actions::ActionEnvelope =
                serde_json::from_value(event["params"].clone()).unwrap();
            assert!(envelope.server_seq > first_seq);
            first_seq = envelope.server_seq;
            if event["params"]["action"]["type"] == "chat/delta" {
                got_text = true;
            }
            if envelope.channel == session {
                got_session = true;
            }
        }
        if event["method"] == "root/sessionSummaryChanged" {
            got_summary = true;
            assert!(event["params"]["changes"].get("createdAt").is_none());
        }
    }
    let mut resumed = connect(&origin, &f.cookie).await;
    let replay=rpc(&mut resumed,3,"reconnect",json!({"channel":ROOT,"clientId":"two","lastSeenServerSeq":baseline,"subscriptions":[ROOT,chat,session]})).await;
    let replay: ahp_types::commands::ReconnectResult =
        serde_json::from_value(replay["result"].clone()).unwrap();
    match replay {
        ahp_types::commands::ReconnectResult::Replay(r) => {
            assert!(r.actions.len() > 4);
            assert!(r.missing.is_empty());
            assert!(r.actions.iter().any(|a| a.channel == chat));
            assert!(r.actions.iter().any(|a| a.channel == session));
        }
        _ => panic!("Expected complete replay"),
    }
    // A different coordinator boot must snapshot, even with a durable client record.
    drop(old_server);
    drop(first);
    drop(resumed);
    let reopened = Store::open(f.dir.path().join("relay.db")).unwrap();
    let state = AppState::new(reopened, (*f.state.config).clone());
    let (origin2, _second_server) = server(&state).await;
    let mut restarted = connect(&origin2, &f.cookie).await;
    let snapshot=rpc(&mut restarted,4,"reconnect",json!({"channel":ROOT,"clientId":"two","lastSeenServerSeq":baseline,"subscriptions":[ROOT,chat,session]})).await;
    match serde_json::from_value::<ahp_types::commands::ReconnectResult>(snapshot["result"].clone())
        .unwrap()
    {
        ahp_types::commands::ReconnectResult::Snapshot(s) => assert_eq!(s.snapshots.len(), 3),
        _ => panic!("Expected snapshot after restart"),
    }
}
struct SdkTransport(Socket);
impl sdk::transport::Transport for SdkTransport {
    async fn send(
        &mut self,
        message: sdk::transport::TransportMessage,
    ) -> std::result::Result<(), sdk::TransportError> {
        let value = message.into_parsed()?;
        self.0
            .send(WsMessage::Text(
                serde_json::to_string(&value)
                    .map_err(|e| sdk::TransportError::Protocol(e.to_string()))?
                    .into(),
            ))
            .await
            .map_err(|e| sdk::TransportError::Io(e.to_string()))
    }
    async fn recv(
        &mut self,
    ) -> std::result::Result<Option<sdk::transport::TransportMessage>, sdk::TransportError> {
        loop {
            match self.0.next().await {
                Some(Ok(WsMessage::Text(t))) => {
                    return Ok(Some(sdk::transport::TransportMessage::Text(t.to_string())));
                }
                Some(Ok(WsMessage::Ping(b))) => self
                    .0
                    .send(WsMessage::Pong(b))
                    .await
                    .map_err(|e| sdk::TransportError::Io(e.to_string()))?,
                Some(Ok(WsMessage::Pong(_))) => {}
                Some(Ok(WsMessage::Close(_))) | None => return Ok(None),
                Some(Err(e)) => return Err(sdk::TransportError::Io(e.to_string())),
                _ => return Err(sdk::TransportError::Protocol("Text frames required".into())),
            }
        }
    }
    async fn close(&mut self) -> std::result::Result<(), sdk::TransportError> {
        self.0
            .close(None)
            .await
            .map_err(|e| sdk::TransportError::Io(e.to_string()))
    }
}
#[tokio::test]
async fn official_sdk_initializes_subscribes_and_receives_canonical_chat_actions() {
    let f = fixture();
    let (origin, _server) = server(&f.state).await;
    let client = sdk::Client::connect(
        SdkTransport(connect(&origin, &f.cookie).await),
        Default::default(),
    )
    .await
    .unwrap();
    client.ping().await.unwrap();
    let initialized = client
        .initialize(
            "official-sdk".into(),
            vec!["1.0.0".into()],
            vec![ROOT.into()],
        )
        .await
        .unwrap();
    assert_eq!(initialized.protocol_version, "1.0.0");
    let chat = format!("ahp-chat:/{}", f.session.id);
    let (result, mut subscription) = client.subscribe(chat).await.unwrap();
    let mut state = match result.snapshot.unwrap().state {
        ahp_types::state::SnapshotState::Chat(c) => *c,
        _ => panic!("Expected chat snapshot"),
    };
    produce(&f, "SDK observed text");
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), subscription.recv())
            .await
            .unwrap()
            .unwrap();
        if let sdk::client::SubscriptionEvent::Action(action) = event {
            assert_eq!(
                sdk::reducers::apply_action_to_chat(&mut state, &action.action),
                sdk::reducers::ReduceOutcome::Applied
            );
            if matches!(
                action.action,
                ahp_types::actions::StateAction::ChatTurnComplete(_)
            ) {
                break;
            }
        }
    }
    assert_eq!(state.turns.len(), 1);
    assert_eq!(
        state.interactivity,
        Some(ahp_types::state::ChatInteractivity::Full)
    );
    assert!(
        matches!(&state.turns[0].response_parts[0],ahp_types::state::ResponsePart::Markdown(p) if p.content=="SDK observed text")
    );
    client.shutdown().await;
}
#[tokio::test]
async fn protocol_authority_origin_revocation_and_unsupported_writes_fail_closed() {
    let f = fixture();
    let (origin, _server) = server(&f.state).await;
    let mut request = format!("{}/v1/web/ahp", origin.replace("http://", "ws://"))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", "http://evil.local".parse().unwrap());
    request
        .headers_mut()
        .insert("cookie", f.cookie.parse().unwrap());
    assert!(connect_async(request).await.is_err());
    let mut socket = connect(&origin, &f.cookie).await;
    let chat = format!("ahp-chat:/{}", f.session.id);
    assert!(
        rpc(&mut socket, 1, "subscribe", json!({"channel":chat}))
            .await
            .get("error")
            .is_some()
    );
    assert_eq!(rpc(&mut socket,2,"initialize",json!({"channel":ROOT,"clientId":"security","protocolVersions":["1.0.0"],"initialSubscriptions":[chat]})).await["result"]["protocolVersion"],"1.0.0");
    let denied=rpc(&mut socket,3,"dispatchAction",json!({"channel":chat,"clientSeq":1,"action":{"type":"chat/turnStarted","turnId":"unapproved","message":{"text":"do work","origin":{"kind":"user"}}}})).await;
    assert_eq!(denied["error"]["code"], -32600);
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_device(&f.device)
        .unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Close(_))) | None => break,
                _ => {}
            }
        }
    })
    .await;
    assert!(closed.is_ok());
}

#[tokio::test]
async fn another_granted_admin_cannot_observe_a_private_session_and_revoked_grants_are_missing() {
    use dunlin_domain::collaboration::{Permission, WorkspaceRole};
    let f = fixture();
    let other_cookie = {
        let store = f.state.store.lock().unwrap();
        let other = store.create_user("other-admin", "admin").unwrap();
        store
            .set_team_member(
                &f.user,
                &f.workspace,
                &other.id,
                WorkspaceRole::Member,
                true,
                None,
            )
            .unwrap();
        store
            .set_team_project_grant(
                &f.user,
                &f.workspace,
                &f.project,
                &other.id,
                BTreeSet::from([Permission::ViewProject]),
                None,
            )
            .unwrap();
        let (device, _) = store.create_device(&other.id, "browser").unwrap();
        format!(
            "dunlin_web_session={}",
            store.create_browser_session(&device.id, 3600).unwrap()
        )
    };
    let (origin, _server) = server(&f.state).await;
    let chat = format!("ahp-chat:/{}", f.session.id);
    let mut stranger = connect(&origin, &other_cookie).await;
    assert!(rpc(&mut stranger,1,"initialize",json!({"channel":ROOT,"clientId":"stranger","protocolVersions":["1.0.0"],"initialSubscriptions":[ROOT]})).await.get("result").is_some());
    assert_eq!(
        rpc(&mut stranger, 2, "listSessions", json!({"channel":ROOT})).await["result"]["items"],
        json!([])
    );
    assert_eq!(
        rpc(&mut stranger, 3, "subscribe", json!({"channel":chat})).await["error"]["code"],
        -32001
    );
    let mut owner = connect(&origin, &f.cookie).await;
    let init=rpc(&mut owner,1,"initialize",json!({"channel":ROOT,"clientId":"revoked-grant","protocolVersions":["1.0.0"],"initialSubscriptions":[ROOT,chat]})).await;
    let seq = init["result"]["serverSeq"].as_u64().unwrap();
    owner.close(None).await.unwrap();
    {
        let store = f.state.store.lock().unwrap();
        let revision = store
            .team_project_context(&f.workspace, &f.project, &f.user)
            .unwrap()
            .project_grant
            .unwrap()
            .revision;
        store
            .set_team_project_grant(
                &f.user,
                &f.workspace,
                &f.project,
                &f.user,
                BTreeSet::new(),
                Some(revision),
            )
            .unwrap();
    }
    let mut resumed = connect(&origin, &f.cookie).await;
    let response=rpc(&mut resumed,3,"reconnect",json!({"channel":ROOT,"clientId":"revoked-grant","lastSeenServerSeq":seq,"subscriptions":[ROOT,chat]})).await;
    assert_eq!(response["result"]["missing"], json!([chat]));
    assert_eq!(response["result"]["actions"], json!([]));
}

#[tokio::test]
async fn unsupported_versions_parse_errors_and_disabled_transcript_transport_are_explicit() {
    let f = fixture();
    let (origin, _server) = server(&f.state).await;
    let mut socket = connect(&origin, &f.cookie).await;
    let version = rpc(
        &mut socket,
        1,
        "initialize",
        json!({"channel":ROOT,"clientId":"version","protocolVersions":["2.0.0"]}),
    )
    .await;
    assert_eq!(version["error"]["code"], -32005);
    assert_eq!(
        version["error"]["data"]["supportedVersions"],
        json!(["1.0.0", "0.9.0"])
    );
    let mut malformed = connect(&origin, &f.cookie).await;
    malformed.send(WsMessage::Text("{".into())).await.unwrap();
    assert_eq!(next(&mut malformed).await["error"]["code"], -32700);
    for transcripts in [true, false] {
        let mut disabled = f.state.clone();
        let config = Arc::make_mut(&mut disabled.config);
        config.experimental_interactive_runtime = !transcripts;
        config.store_transcripts = transcripts;
        let (origin, _server) = server(&disabled).await;
        let mut request = format!("{}/v1/web/ahp", origin.replace("http://", "ws://"))
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
        request
            .headers_mut()
            .insert("cookie", f.cookie.parse().unwrap());
        match connect_async(request).await {
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                assert_eq!(response.status(), StatusCode::NOT_FOUND)
            }
            _ => panic!("Disabled observer transport must not upgrade"),
        }
    }
}

#[tokio::test]
async fn replay_over_budget_returns_one_consistent_snapshot_cut() {
    let f = fixture();
    let (origin, _server) = server(&f.state).await;
    let chat = format!("ahp-chat:/{}", f.session.id);
    let session = format!("ahp-session:/{}", f.session.id);
    let mut socket = connect(&origin, &f.cookie).await;
    let initialized=rpc(&mut socket,1,"initialize",json!({"channel":ROOT,"clientId":"gap","protocolVersions":["1.0.0"],"initialSubscriptions":[ROOT,chat,session]})).await;
    let before = initialized["result"]["serverSeq"].as_u64().unwrap();
    socket.close(None).await.unwrap();
    {
        let store = f.state.store.lock().unwrap();
        let a = store
            .team_project_context(&f.workspace, &f.project, &f.user)
            .unwrap();
        let j = store.interactive_journal();
        let t = j.enqueue(&a, &f.session.id, "gap", "prompt").unwrap();
        j.claim_next(&a, &f.session.id, &f.lease).unwrap();
        j.transition(&a, &f.session.id, &f.lease, &t, TurnPhase::Running)
            .unwrap();
        for _ in 0..130 {
            j.output(&a, &f.session.id, &f.lease, &t, "chunk").unwrap();
        }
        j.transition(&a, &f.session.id, &f.lease, &t, TurnPhase::Completed)
            .unwrap();
    }
    let mut socket = connect(&origin, &f.cookie).await;
    let response=rpc(&mut socket,2,"reconnect",json!({"channel":ROOT,"clientId":"gap","lastSeenServerSeq":before,"subscriptions":[ROOT,chat,session]})).await;
    assert_eq!(response["result"]["type"], "snapshot");
    let snapshots = response["result"]["snapshots"].as_array().unwrap();
    assert_eq!(snapshots.len(), 3);
    assert!(
        snapshots
            .iter()
            .all(|s| s["fromSeq"] == snapshots[0]["fromSeq"])
    );
}
#[tokio::test]
async fn session_pages_have_connection_scoped_cursors_and_omit_an_absent_next_cursor() {
    let f = fixture();
    {
        let store = f.state.store.lock().unwrap();
        let a = store
            .team_project_context(&f.workspace, &f.project, &f.user)
            .unwrap();
        for _ in 0..2 {
            store
                .interactive_journal()
                .create(&a, &f.workspace, &f.project)
                .unwrap();
        }
    }
    let (origin, _server) = server(&f.state).await;
    let mut socket = connect(&origin, &f.cookie).await;
    rpc(
        &mut socket,
        1,
        "initialize",
        json!({"channel":ROOT,"clientId":"pages","protocolVersions":["1.0.0"]}),
    )
    .await;
    let first = rpc(
        &mut socket,
        2,
        "listSessions",
        json!({"channel":ROOT,"limit":1}),
    )
    .await;
    let cursor = first["result"]["nextCursor"].as_str().unwrap();
    let mut other = connect(&origin, &f.cookie).await;
    rpc(
        &mut other,
        1,
        "initialize",
        json!({"channel":ROOT,"clientId":"pages-other","protocolVersions":["1.0.0"]}),
    )
    .await;
    assert_eq!(
        rpc(
            &mut other,
            2,
            "listSessions",
            json!({"channel":ROOT,"cursor":cursor,"limit":1})
        )
        .await["error"]["code"],
        -32602
    );
    let second = rpc(
        &mut socket,
        3,
        "listSessions",
        json!({"channel":ROOT,"cursor":cursor,"limit":1}),
    )
    .await;
    let third = rpc(
        &mut socket,
        4,
        "listSessions",
        json!({"channel":ROOT,"cursor":second["result"]["nextCursor"],"limit":1}),
    )
    .await;
    assert!(third["result"].get("nextCursor").is_none());
    let ids = BTreeSet::from([
        first["result"]["items"][0]["resource"].as_str().unwrap(),
        second["result"]["items"][0]["resource"].as_str().unwrap(),
        third["result"]["items"][0]["resource"].as_str().unwrap(),
    ]);
    assert_eq!(ids.len(), 3);
}

#[test]
fn failed_subscribe_keeps_pending_actions_for_existing_subscriptions() {
    for oversized in [false, true] {
        let f = fixture();
        let chat = format!("ahp-chat:/{}", f.session.id);
        let token = f.cookie.strip_prefix("dunlin_web_session=").unwrap();
        let mut conn = Connection::default();
        handle(&mut conn,&f.state,token,"initialize",json!({"channel":ROOT,"clientId":"subscribe-cut","protocolVersions":["1.0.0"],"initialSubscriptions":[ROOT,chat]})).unwrap();
        let before = conn.cursor;
        produce(&f, "must survive a failed subscribe");
        let target = if oversized {
            let store = f.state.store.lock().unwrap();
            let a = store
                .team_project_context(&f.workspace, &f.project, &f.user)
                .unwrap();
            let j = store.interactive_journal();
            let session = j.create(&a, &f.workspace, &f.project).unwrap();
            for n in 0..8 {
                j.enqueue(
                    &a,
                    &session.id,
                    &format!("big-{n}"),
                    &format!("x{}", "\u{0001}".repeat(32760)),
                )
                .unwrap();
            }
            format!("ahp-chat:/{}", session.id)
        } else {
            "ahp-chat:/not-authorized".into()
        };
        assert!(
            handle(
                &mut conn,
                &f.state,
                token,
                "subscribe",
                json!({"channel":target})
            )
            .is_err()
        );
        assert_eq!(conn.cursor, before);
        let updates = {
            let store = f.state.store.lock().unwrap();
            refresh(&mut conn, &f.state, &store, &f.user).unwrap()
        };
        assert!(updates.iter().any(|v| v["method"] == "action"
            && v["params"]["action"]["content"] == "must survive a failed subscribe"));
        assert!(!conn.channels.contains(&target));
    }
}

#[tokio::test]
async fn dispatch_notification_rejection_is_durable_replayed_and_never_enqueued() {
    let f = fixture();
    let (origin, _server) = server(&f.state).await;
    let chat = format!("ahp-chat:/{}", f.session.id);
    let mut a = connect(&origin, &f.cookie).await;
    let mut b = connect(&origin, &f.cookie).await;
    for (socket, client) in [(&mut a, "dispatch-a"), (&mut b, "dispatch-b")] {
        assert!(rpc(socket,1,"initialize",json!({"channel":ROOT,"clientId":client,"protocolVersions":["1.0.0"],"initialSubscriptions":[ROOT,chat]})).await.get("error").is_none());
    }
    let params = json!({"channel":chat,"clientSeq":1,"action":{"type":"chat/pendingMessageSet","kind":"queued","id":"client-message","message":{"text":"must not start a missing actor","origin":{"kind":"user"}}}});
    a.send(WsMessage::Text(
        json!({"jsonrpc":"2.0","method":"dispatchAction","params":params})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    async fn echo(s: &mut Socket) -> Value {
        for _ in 0..32 {
            let v = next(s).await;
            if v["method"] == "action" && v["params"]["origin"]["clientId"] == "dispatch-a" {
                return v["params"].clone();
            }
        }
        panic!("No dispatch echo");
    }
    let first = echo(&mut a).await;
    assert_eq!(first, echo(&mut b).await);
    assert_eq!(first["origin"]["clientSeq"], 1);
    assert_eq!(first["action"], params["action"]);
    assert!(
        first["rejectionReason"]
            .as_str()
            .unwrap()
            .contains("runtime")
    );
    let clock = f
        .state
        .store
        .lock()
        .unwrap()
        .interactive_journal()
        .channel_clock()
        .unwrap()
        .sequence;
    a.send(WsMessage::Text(
        json!({"jsonrpc":"2.0","method":"dispatchAction","params":params})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(echo(&mut a).await, first);
    let store = f.state.store.lock().unwrap();
    assert_eq!(
        store
            .interactive_journal()
            .channel_clock()
            .unwrap()
            .sequence,
        clock
    );
    let authority = store
        .team_project_context(&f.workspace, &f.project, &f.user)
        .unwrap();
    let (_, turns) = store
        .interactive_journal()
        .snapshot(&authority, &f.session.id)
        .unwrap();
    assert!(turns.is_empty());
}

#[tokio::test]
async fn official_sdk_dispatch_receives_rejection_envelope_without_reducing_state() {
    let f = fixture();
    let (origin, _server) = server(&f.state).await;
    let client = sdk::Client::connect(
        SdkTransport(connect(&origin, &f.cookie).await),
        Default::default(),
    )
    .await
    .unwrap();
    client
        .initialize(
            "dispatch-sdk".into(),
            vec!["1.0.0".into()],
            vec![ROOT.into()],
        )
        .await
        .unwrap();
    let chat = format!("ahp-chat:/{}", f.session.id);
    let (_, mut subscription) = client.subscribe(chat.clone()).await.unwrap();
    let action=serde_json::from_value(json!({"type":"chat/pendingMessageSet","kind":"queued","id":"sdk-message","message":{"text":"SDK queued text","origin":{"kind":"user"}}})).unwrap();
    let dispatched = client.dispatch(chat, action).await.unwrap();
    loop {
        let e = tokio::time::timeout(Duration::from_secs(5), subscription.recv())
            .await
            .unwrap()
            .unwrap();
        if let sdk::client::SubscriptionEvent::Action(a) = e {
            assert_eq!(a.origin.as_ref().unwrap().client_id, "dispatch-sdk");
            assert_eq!(a.origin.as_ref().unwrap().client_seq, dispatched.client_seq);
            assert!(a.rejection_reason.is_some());
            break;
        }
    }
    client.shutdown().await;
}

#[tokio::test]
async fn shared_reader_visibility_pins_survive_disconnect_and_fence_revoke_regrant_aba() {
    use dunlin_domain::collaboration::{Permission, WorkspaceRole};

    async fn expect_scope_close(socket: &mut Socket, expected: u16) {
        let code = tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                match socket.next().await {
                    Some(Ok(WsMessage::Close(Some(frame)))) => return u16::from(frame.code),
                    Some(Ok(WsMessage::Ping(bytes))) => {
                        socket.send(WsMessage::Pong(bytes)).await.unwrap();
                    }
                    Some(Ok(WsMessage::Text(text))) => {
                        panic!("Stale visibility received data before closing: {text}");
                    }
                    other => panic!("Expected an explicit scope close, received {other:?}"),
                }
            }
        })
        .await
        .expect("Visibility changes must close the old socket promptly");
        assert_eq!(code, expected);
    }

    let f = fixture();
    produce(&f, "shared visibility snapshot marker");
    let (reader, device, cookie) = {
        let store = f.state.store.lock().unwrap();
        let reader = store.create_user("visibility-reader", "admin").unwrap();
        store
            .set_team_member(
                &f.user,
                &f.workspace,
                &reader.id,
                WorkspaceRole::Member,
                true,
                None,
            )
            .unwrap();
        // Even a reader with a current CreateSession grant needs an explicit
        // read share. The visibility key must represent that share, not its role.
        store
            .set_team_project_grant(
                &f.user,
                &f.workspace,
                &f.project,
                &reader.id,
                BTreeSet::from([
                    Permission::ViewProject,
                    Permission::ViewSharedSession,
                    Permission::CreateSession,
                ]),
                None,
            )
            .unwrap();
        let shared = store
            .replace_session_readers(&f.user, &f.session.id, 0, std::slice::from_ref(&reader.id))
            .unwrap();
        assert_eq!(shared.revision, 1);
        let (device, _) = store
            .create_device(&reader.id, "visibility-browser")
            .unwrap();
        let cookie = format!(
            "dunlin_web_session={}",
            store.create_browser_session(&device.id, 3600).unwrap()
        );
        (reader.id, device.id, cookie)
    };
    let (origin, _server) = server(&f.state).await;
    let chat = format!("ahp-chat:/{}", f.session.id);
    let session = format!("ahp-session:/{}", f.session.id);
    let client = "durable-shared-visibility";
    let mut old = connect(&origin, &cookie).await;
    let initialized = rpc(
        &mut old,
        1,
        "initialize",
        json!({"channel":ROOT,"clientId":client,"protocolVersions":["1.0.0"],"initialSubscriptions":[ROOT,chat,session]}),
    )
    .await;
    let baseline = initialized["result"]["serverSeq"].as_u64().unwrap();
    let original_pins = {
        let store = f.state.store.lock().unwrap();
        let registered = store.ahp_client(&device, client).unwrap().unwrap();
        assert_eq!(registered.boot, f.state.browser_sessions.boot);
        assert_eq!(registered.visibility.len(), 2);
        assert_eq!(
            registered.visibility[&chat],
            store.ahp_access(&reader, &chat).unwrap().read_key
        );
        registered.visibility
    };
    let new_pins = {
        // Keep the mutex through both changes: the transport cannot observe the
        // intermediate denial. Only the grant revision can expose this ABA.
        let store = f.state.store.lock().unwrap();
        assert_eq!(
            store
                .replace_session_readers(&f.user, &f.session.id, 1, &[])
                .unwrap()
                .revision,
            2
        );
        assert_eq!(
            store
                .replace_session_readers(&f.user, &f.session.id, 2, std::slice::from_ref(&reader),)
                .unwrap()
                .revision,
            3
        );
        assert_eq!(
            store
                .interactive_journal()
                .channel_clock()
                .unwrap()
                .sequence,
            baseline,
            "An ACL change must not masquerade as an AHP replay gap"
        );
        let current = visibility(
            &accesses(
                &store,
                &reader,
                &BTreeSet::from([chat.clone(), session.clone()]),
            )
            .unwrap(),
        );
        assert_ne!(current, original_pins);
        assert_eq!(
            store
                .ahp_client(&device, client)
                .unwrap()
                .unwrap()
                .visibility,
            original_pins,
            "Closing the old connection must not refresh its durable pins"
        );
        current
    };
    expect_scope_close(&mut old, 1013).await;

    let mut resumed = connect(&origin, &cookie).await;
    let recovered = rpc(
        &mut resumed,
        2,
        "reconnect",
        json!({"channel":ROOT,"clientId":client,"lastSeenServerSeq":baseline,"subscriptions":[ROOT,chat,session]}),
    )
    .await;
    assert_eq!(recovered["result"]["type"], "snapshot");
    let snapshots = recovered["result"]["snapshots"].as_array().unwrap();
    assert_eq!(snapshots.len(), 3);
    for resource in [ROOT, chat.as_str(), session.as_str()] {
        let snapshot = snapshots
            .iter()
            .find(|s| s["resource"] == resource)
            .unwrap();
        assert_eq!(snapshot["fromSeq"], baseline);
    }
    assert!(
        snapshots.iter().find(|s| s["resource"] == chat).unwrap()["state"]
            .to_string()
            .contains("shared visibility snapshot marker")
    );
    {
        let store = f.state.store.lock().unwrap();
        let registered = store.ahp_client(&device, client).unwrap().unwrap();
        assert_eq!(registered.boot, f.state.browser_sessions.boot);
        assert_eq!(registered.visibility, new_pins);
        assert_eq!(
            store
                .replace_session_readers(&f.user, &f.session.id, 3, &[])
                .unwrap()
                .revision,
            4
        );
        assert!(store.ahp_access(&reader, &chat).is_err());
    }
    expect_scope_close(&mut resumed, 1008).await;
}
