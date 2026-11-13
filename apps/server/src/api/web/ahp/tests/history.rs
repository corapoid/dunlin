use super::*;
use ahp_types::commands::{FetchTurnsParams, FetchTurnsResult, SubscribeView};

async fn initialize_root(socket: &mut Socket, client: &str) {
    let result=rpc(socket,1,"initialize",json!({"channel":ROOT,"clientId":client,"protocolVersions":["1.0.0"],"initialSubscriptions":[ROOT]})).await;
    assert!(result.get("error").is_none(), "{result}");
}
async fn fetch_frame(socket: &mut Socket, id: u64, channel: &str, cursor: &str) -> (Value, Value) {
    socket.send(WsMessage::Text(json!({"jsonrpc":"2.0","id":id,"method":"fetchTurns","params":{"channel":channel,"cursor":cursor}}).to_string().into())).await.unwrap();
    let action = next(socket).await;
    assert_eq!(
        action["method"], "action",
        "The page must precede its RPC result: {action}"
    );
    assert_eq!(action["params"]["action"]["type"], "chat/turnsLoaded");
    let response = next(socket).await;
    assert_eq!(response["id"], id);
    assert_eq!(response["result"], json!({}));
    (action, response)
}

#[tokio::test]
async fn real_ws_history_is_requester_only_ordered_single_use_and_not_canonical() {
    let f = fixture();
    let ids = (0..8)
        .map(|i| produce(&f, &format!("historical marker {i}")))
        .collect::<Vec<_>>();
    let (origin, _server) = server(&f.state).await;
    let chat = format!("ahp-chat:/{}", f.session.id);
    let mut first = connect(&origin, &f.cookie).await;
    let mut other = connect(&origin, &f.cookie).await;
    initialize_root(&mut first, "history-first").await;
    initialize_root(&mut other, "history-other").await;
    let a = rpc(
        &mut first,
        2,
        "subscribe",
        json!({"channel":chat,"view":{"turns":2}}),
    )
    .await;
    let b = rpc(
        &mut other,
        2,
        "subscribe",
        json!({"channel":chat,"view":{"turns":2}}),
    )
    .await;
    assert_eq!(
        a["result"]["snapshot"]["state"]["turns"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let cursor = a["result"]["snapshot"]["state"]["turnsNextCursor"]
        .as_str()
        .unwrap();
    assert_ne!(
        Some(cursor),
        b["result"]["snapshot"]["state"]["turnsNextCursor"].as_str()
    );
    let foreign = rpc(
        &mut other,
        3,
        "fetchTurns",
        json!({"channel":chat,"cursor":cursor}),
    )
    .await;
    assert_eq!(foreign["error"]["code"], -32602);
    let clock = f
        .state
        .store
        .lock()
        .unwrap()
        .interactive_journal()
        .channel_clock()
        .unwrap()
        .sequence;
    let (loaded, _) = fetch_frame(&mut first, 3, &chat, cursor).await;
    let seq = loaded["params"]["serverSeq"].as_u64().unwrap();
    assert!(seq > clock);
    let turns = loaded["params"]["action"]["turns"].as_array().unwrap();
    assert_eq!(
        turns
            .iter()
            .map(|turn| turn["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>(),
        ids[..6]
    );
    assert!(loaded["params"]["action"].get("turnsNextCursor").is_none());
    let ping = next_after_ping(&mut other, 4).await;
    assert_eq!(
        ping["id"], 4,
        "An unrelated connection must not receive the page/cursor"
    );
    let reused = rpc(
        &mut first,
        4,
        "fetchTurns",
        json!({"channel":chat,"cursor":cursor}),
    )
    .await;
    assert_eq!(reused["error"]["code"], -32602);
    let store = f.state.store.lock().unwrap();
    let access = store.ahp_access(&f.user, &chat).unwrap();
    let j = store.interactive_journal();
    assert_eq!(
        j.channel_clock().unwrap().sequence,
        seq,
        "Invalid cursor must not reserve another slot"
    );
    assert!(
        j.projected_chat(&access.context, &access.id)
            .unwrap()
            .turns_next_cursor
            .is_none()
    );
    let replay = j
        .channel_replay(
            &[ChannelSubscription {
                context: &access.context,
                session: &access.id,
                chat: true,
            }],
            clock,
        )
        .unwrap();
    assert!(
        matches!(replay,ChannelReplay::Actions {actions,..} if actions.is_empty()),
        "Page payloads must never be replayed to another observer"
    );
}
async fn next_after_ping(socket: &mut Socket, id: u64) -> Value {
    socket
        .send(WsMessage::Text(
            json!({"jsonrpc":"2.0","id":id,"method":"ping","params":{"channel":ROOT}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    next(socket).await
}

#[tokio::test]
async fn official_sdk_view_and_fetch_turns_reduce_with_live_suffix_at_a_fixed_historical_cut() {
    let f = fixture();
    let mut ids = (0..8)
        .map(|i| produce(&f, &format!("sdk history {i}")))
        .collect::<Vec<_>>();
    let (origin, _server) = server(&f.state).await;
    let client = sdk::Client::connect(
        SdkTransport(connect(&origin, &f.cookie).await),
        Default::default(),
    )
    .await
    .unwrap();
    client
        .initialize("paged-sdk".into(), vec!["1.0.0".into()], vec![ROOT.into()])
        .await
        .unwrap();
    let chat = format!("ahp-chat:/{}", f.session.id);
    let (subscribed, mut subscription) = client
        .subscribe_with_options(chat.clone(), None, Some(SubscribeView { turns: Some(2) }))
        .await
        .unwrap();
    let snapshot = subscribed.snapshot.unwrap();
    let baseline = snapshot.from_seq as u64;
    let ahp_types::state::SnapshotState::Chat(mut state) = snapshot.state else {
        panic!("chat")
    };
    assert_eq!(state.turns.len(), 2);
    let cursor = state.turns_next_cursor.clone().unwrap();
    let live = produce(&f, "live after frozen cut");
    ids.push(live.clone());
    let _: FetchTurnsResult = client
        .request(
            "fetchTurns",
            FetchTurnsParams {
                channel: chat,
                cursor: Some(cursor),
                meta: None,
            },
        )
        .await
        .unwrap();
    let mut page_seen = false;
    let mut last = baseline;
    while !page_seen {
        let event = tokio::time::timeout(Duration::from_secs(5), subscription.recv())
            .await
            .unwrap()
            .unwrap();
        if let sdk::client::SubscriptionEvent::Action(envelope) = event {
            assert!(envelope.server_seq > last);
            last = envelope.server_seq;
            if let ahp_types::actions::StateAction::ChatTurnsLoaded(page) = &envelope.action {
                assert!(!page.turns.iter().any(|turn| turn.id == live));
                page_seen = true;
            }
            assert_eq!(
                sdk::reducers::apply_action_to_chat(&mut state, &envelope.action),
                sdk::reducers::ReduceOutcome::Applied
            );
        }
    }
    assert_eq!(
        state
            .turns
            .iter()
            .map(|turn| turn.id.clone())
            .collect::<Vec<_>>(),
        ids
    );
    assert!(state.turns_next_cursor.is_none());
    client.shutdown().await;
}

#[tokio::test]
async fn persisted_paged_view_reconnects_with_a_new_bounded_tail_and_new_cursor() {
    let f = fixture();
    for i in 0..65 {
        produce(&f, &format!("{i:03}{}", "x".repeat(16 * 1024)));
    }
    let (origin, _server) = server(&f.state).await;
    let chat = format!("ahp-chat:/{}", f.session.id);
    let mut old = connect(&origin, &f.cookie).await;
    initialize_root(&mut old, "durable-history-view").await;
    let all = rpc(&mut old, 2, "subscribe", json!({"channel":chat})).await;
    assert_eq!(
        all["error"]["code"], -32602,
        "No view must not silently tail large retained history"
    );
    let tail = rpc(
        &mut old,
        3,
        "subscribe",
        json!({"channel":chat,"view":{"turns":2}}),
    )
    .await;
    let cursor = tail["result"]["snapshot"]["state"]["turnsNextCursor"]
        .as_str()
        .unwrap()
        .to_string();
    let cut = tail["result"]["snapshot"]["fromSeq"].as_u64().unwrap();
    old.close(None).await.unwrap();
    let mut resumed = connect(&origin, &f.cookie).await;
    let recovered=rpc(&mut resumed,4,"reconnect",json!({"channel":ROOT,"clientId":"durable-history-view","lastSeenServerSeq":cut,"subscriptions":[ROOT,chat]})).await;
    assert_eq!(recovered["result"]["type"], "snapshot");
    let snapshot = recovered["result"]["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|snapshot| snapshot["resource"] == chat)
        .unwrap();
    assert_eq!(snapshot["state"]["turns"].as_array().unwrap().len(), 2);
    assert_ne!(
        snapshot["state"]["turnsNextCursor"].as_str(),
        Some(cursor.as_str())
    );
    let stale = rpc(
        &mut resumed,
        5,
        "fetchTurns",
        json!({"channel":chat,"cursor":cursor}),
    )
    .await;
    assert_eq!(stale["error"]["code"], -32602);
    fetch_frame(
        &mut resumed,
        6,
        &chat,
        snapshot["state"]["turnsNextCursor"].as_str().unwrap(),
    )
    .await;
}

#[test]
fn history_expiry_resubscribe_unsubscribe_and_invalid_fetch_keep_view_state_consistent() {
    let f = fixture();
    for i in 0..4 {
        produce(&f, &format!("expiry {i}"));
    }
    let token = f.cookie.split_once('=').unwrap().1;
    let chat = format!("ahp-chat:/{}", f.session.id);
    let mut conn = Connection::default();
    handle(&mut conn,&f.state,token,"initialize",json!({"channel":ROOT,"clientId":"expiry-history","protocolVersions":["1.0.0"],"initialSubscriptions":[ROOT]})).unwrap();
    let reply = handle(
        &mut conn,
        &f.state,
        token,
        "subscribe",
        json!({"channel":chat,"view":{"turns":0}}),
    )
    .unwrap();
    let cursor = reply.result["snapshot"]["state"]["turnsNextCursor"]
        .as_str()
        .unwrap()
        .to_string();
    conn.history.get_mut(&chat).unwrap().created =
        Instant::now() - HISTORY_TTL - Duration::from_secs(1);
    assert_eq!(
        handle(
            &mut conn,
            &f.state,
            token,
            "fetchTurns",
            json!({"channel":chat,"cursor":cursor})
        )
        .unwrap_err()
        .0,
        -32602
    );
    assert!(conn.history.is_empty());
    assert_eq!(conn.views[&chat], 0);
    assert_eq!(
        handle(
            &mut conn,
            &f.state,
            token,
            "fetchTurns",
            json!({"channel":chat})
        )
        .unwrap_err()
        .0,
        -32602
    );
    let renewed = handle(
        &mut conn,
        &f.state,
        token,
        "subscribe",
        json!({"channel":chat,"view":{"turns":1}}),
    )
    .unwrap();
    assert_ne!(
        renewed.result["snapshot"]["state"]["turnsNextCursor"].as_str(),
        Some(cursor.as_str())
    );
    let before = conn.cursor;
    produce(&f, "pending live update");
    assert_eq!(
        handle(
            &mut conn,
            &f.state,
            token,
            "fetchTurns",
            json!({"channel":chat,"cursor":"not-a-real-cursor"})
        )
        .unwrap_err()
        .0,
        -32602
    );
    assert_eq!(
        conn.cursor, before,
        "Invalid RPC staged refresh must roll back instead of losing live actions"
    );
    let store = f.state.store.lock().unwrap();
    let replay = refresh(&mut conn, &f.state, &store, &f.user).unwrap();
    assert!(replay.iter().any(|message| message["method"] == "action"));
    drop(store);
    handle(
        &mut conn,
        &f.state,
        token,
        "unsubscribe",
        json!({"channel":chat}),
    )
    .unwrap();
    assert!(!conn.views.contains_key(&chat));
    assert!(conn.history.is_empty());
    assert!(
        f.state
            .store
            .lock()
            .unwrap()
            .ahp_client(&f.device, "expiry-history")
            .unwrap()
            .unwrap()
            .views
            .is_empty()
    );
}

#[tokio::test]
async fn shared_reader_history_is_wire_read_only_and_share_aba_fences_the_frozen_cut() {
    use dunlin_domain::collaboration::{Permission, WorkspaceRole};
    let f = fixture();
    for i in 0..6 {
        produce(&f, &format!("shared older {i}"));
    }
    let (reader, device, cookie) = {
        let store = f.state.store.lock().unwrap();
        let reader = store.create_user("history-reader", "member").unwrap();
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
        store
            .set_team_project_grant(
                &f.user,
                &f.workspace,
                &f.project,
                &reader.id,
                [
                    Permission::ViewProject,
                    Permission::ViewSharedSession,
                    Permission::CreateSession,
                ]
                .into_iter()
                .collect(),
                None,
            )
            .unwrap();
        store
            .replace_session_readers(&f.user, &f.session.id, 0, std::slice::from_ref(&reader.id))
            .unwrap();
        let (device, _) = store.create_device(&reader.id, "history-reader").unwrap();
        let token = store.create_browser_session(&device.id, 3600).unwrap();
        (reader.id, device.id, format!("dunlin_web_session={token}"))
    };
    let (origin, _server) = server(&f.state).await;
    let chat = format!("ahp-chat:/{}", f.session.id);
    let session = format!("ahp-session:/{}", f.session.id);
    let mut socket = connect(&origin, &cookie).await;
    initialize_root(&mut socket, "shared-history-wire").await;
    let tail = rpc(
        &mut socket,
        2,
        "subscribe",
        json!({"channel":chat,"view":{"turns":1}}),
    )
    .await;
    let mut state: ahp_types::state::ChatState =
        serde_json::from_value(tail["result"]["snapshot"]["state"].clone()).unwrap();
    assert_eq!(
        state.interactivity,
        Some(ahp_types::state::ChatInteractivity::ReadOnly)
    );
    assert_eq!(state.meta.as_ref().unwrap()["dunlin.observerOnly"], true);
    for key in [
        "dunlin.queueText",
        "dunlin.controlTurns",
        "dunlin.approveTools",
    ] {
        assert_eq!(state.meta.as_ref().unwrap()[key], false);
    }
    let details = rpc(&mut socket, 3, "subscribe", json!({"channel":session})).await;
    assert_eq!(
        details["result"]["snapshot"]["state"]["chats"][0]["interactivity"],
        "read-only"
    );
    let listed = rpc(&mut socket, 4, "listSessions", json!({"channel":ROOT})).await;
    assert_eq!(
        listed["result"]["items"][0]["chats"][0]["interactivity"],
        "read-only"
    );
    assert_eq!(
        listed["result"]["items"][0]["_meta"]["dunlin.queueText"],
        false
    );
    let cursor = state.turns_next_cursor.clone().unwrap();
    let (loaded, _) = fetch_frame(&mut socket, 5, &chat, &cursor).await;
    let envelope: ahp_types::actions::ActionEnvelope =
        serde_json::from_value(loaded["params"].clone()).unwrap();
    assert_eq!(
        sdk::reducers::apply_action_to_chat(&mut state, &envelope.action),
        sdk::reducers::ReduceOutcome::Applied
    );
    assert_eq!(state.turns.len(), 6);
    assert_eq!(
        state.interactivity,
        Some(ahp_types::state::ChatInteractivity::ReadOnly)
    );
    socket.close(None).await.unwrap();

    let token = cookie.split_once('=').unwrap().1;
    let mut conn = Connection::default();
    handle(&mut conn,&f.state,token,"initialize",json!({"channel":ROOT,"clientId":"reader-frozen-epoch","protocolVersions":["1.0.0"],"initialSubscriptions":[ROOT]})).unwrap();
    let old = handle(
        &mut conn,
        &f.state,
        token,
        "subscribe",
        json!({"channel":chat,"view":{"turns":1}}),
    )
    .unwrap();
    let old_cursor = old.result["snapshot"]["state"]["turnsNextCursor"]
        .as_str()
        .unwrap()
        .to_string();
    let denied = handle(
        &mut conn,
        &f.state,
        token,
        "dispatchAction",
        json!({"channel":chat,"clientSeq":1,"action":{"type":"chat/pendingMessageSet","kind":"queued","id":"reader-write","message":{"text":"cannot write shared history","origin":{"kind":"user"}}}}),
    );
    assert_eq!(
        denied.unwrap_err().0,
        -32009,
        "CreateSession on a project does not give a shared reader the owner's controls"
    );
    {
        let store = f.state.store.lock().unwrap();
        store
            .replace_session_readers(&f.user, &f.session.id, 1, &[])
            .unwrap();
        store
            .replace_session_readers(&f.user, &f.session.id, 2, std::slice::from_ref(&reader))
            .unwrap();
    }
    assert_eq!(
        handle(
            &mut conn,
            &f.state,
            token,
            "fetchTurns",
            json!({"channel":chat,"cursor":old_cursor})
        )
        .unwrap_err()
        .0,
        -32011
    );
    assert!(
        conn.history.is_empty(),
        "Visibility loss must release old frozen history buffers"
    );
    let mut fresh = Connection::default();
    handle(&mut fresh,&f.state,token,"initialize",json!({"channel":ROOT,"clientId":"reader-fresh-epoch","protocolVersions":["1.0.0"],"initialSubscriptions":[ROOT]})).unwrap();
    let new = handle(
        &mut fresh,
        &f.state,
        token,
        "subscribe",
        json!({"channel":chat,"view":{"turns":1}}),
    )
    .unwrap();
    let new_cursor = new.result["snapshot"]["state"]["turnsNextCursor"]
        .as_str()
        .unwrap();
    assert_ne!(new_cursor, old_cursor);
    assert_eq!(
        handle(
            &mut fresh,
            &f.state,
            token,
            "fetchTurns",
            json!({"channel":chat,"cursor":old_cursor})
        )
        .unwrap_err()
        .0,
        -32602
    );
    let page = handle(
        &mut fresh,
        &f.state,
        token,
        "fetchTurns",
        json!({"channel":chat,"cursor":new_cursor}),
    )
    .unwrap();
    assert_eq!(
        page.before.last().unwrap()["params"]["action"]["turns"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(
        f.state
            .store
            .lock()
            .unwrap()
            .ahp_client(&device, "reader-fresh-epoch")
            .unwrap()
            .unwrap()
            .views[&chat],
        1
    );
}
