//! Experimental AHP 1.0 transport with bounded text queue dispatch.
//! Other writes and approvals remain unavailable.
use super::*;
use ahp_types::commands::{
    DispatchActionParams, FetchTurnsParams, InitializeParams, ListSessionsParams, ReconnectParams,
    SubscribeParams,
};
use ahp_types::state::{SessionSummary, Snapshot};
use dunlin_session_runtime::{ChannelReplay, ChannelSubscription};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
const ROOT: &str = "ahp-root://";
const FRAME: usize = 1024 * 1024;
#[derive(Debug)]
struct Fault(i64, &'static str);
type Result<T> = std::result::Result<T, Fault>;
fn invalid() -> Fault {
    Fault(-32602, "Invalid AHP parameters")
}
fn unavailable() -> Fault {
    Fault(-32603, "AHP resource unavailable")
}
fn root(seq: u64) -> Result<Snapshot> {
    serde_json::from_value(json!({"resource":ROOT,"fromSeq":seq,"state":{"agents":[{"provider":"dunlin-acp","displayName":"Dunlin ACP","description":"Server-owned ACP sessions","models":[]}],"_meta":{"dunlin.observerOnly":false,"dunlin.queueText":true,"dunlin.controlTurns":true,"dunlin.approveTools":true}}})).map_err(|_|unavailable())
}
#[derive(Default, Clone)]
struct Connection {
    client: Option<String>,
    channels: BTreeSet<String>,
    cursor: u64,
    summaries: BTreeMap<String, SessionSummary>,
    pages: HashMap<String, Vec<String>>,
    visibility: BTreeMap<String, String>,
    views: BTreeMap<String, usize>,
    history: HashMap<String, HistoryWindow>,
    history_finished: BTreeSet<String>,
}
#[derive(Clone)]
struct HistoryWindow {
    frozen: dunlin_session_runtime::FrozenChatHistory,
    before: usize,
    token: String,
    user: String,
    device: String,
    read_key: String,
    created: Instant,
}
const HISTORY_TTL: Duration = Duration::from_secs(300);
fn history_budget(history: &HashMap<String, HistoryWindow>) -> Result<()> {
    if history.len() > 32
        || history.values().map(|h| h.frozen.bytes).sum::<usize>()
            > dunlin_session_runtime::HISTORY_BYTES
        || history
            .values()
            .map(|h| h.frozen.turn_count())
            .sum::<usize>()
            > dunlin_session_runtime::HISTORY_TURNS
    {
        return Err(Fault(
            -32602,
            "Connection history budget exceeded; unsubscribe another chat",
        ));
    }
    Ok(())
}
#[derive(Debug)]
struct Reply {
    result: Value,
    before: Vec<Value>,
}
fn shared_meta(meta: &mut Option<ahp_types::common::JsonObject>, owner: &str) {
    let meta = meta.get_or_insert_with(Default::default);
    for (key, value) in [
        ("dunlin.observerOnly", true),
        ("dunlin.queueText", false),
        ("dunlin.controlTurns", false),
        ("dunlin.approveTools", false),
        ("dunlin.shared", true),
    ] {
        meta.insert(key.into(), json!(value));
    }
    meta.insert("dunlin.ownerId".into(), json!(owner));
}
fn reader_actions(
    store: &crate::store::Store,
    user: &str,
    actions: &mut [ahp_types::actions::ActionEnvelope],
) -> Result<()> {
    for envelope in actions {
        if let ahp_types::actions::StateAction::SessionMetaChanged(action) = &mut envelope.action {
            let id = envelope
                .channel
                .strip_prefix("ahp-session:/")
                .ok_or_else(unavailable)?;
            let (_, _, owner) = store.interactive_identity(id).map_err(|_| unavailable())?;
            if owner != user {
                shared_meta(&mut action.meta, &owner);
            }
        }
    }
    Ok(())
}
fn action_messages(actions: Vec<ahp_types::actions::ActionEnvelope>) -> Vec<Value> {
    actions
        .into_iter()
        .map(|a| json!({"jsonrpc":"2.0","method":"action","params":a}))
        .collect()
}
fn valid_client(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control)
}
fn accesses(
    store: &crate::store::Store,
    user: &str,
    channels: &BTreeSet<String>,
) -> Result<Vec<crate::store::AhpAccess>> {
    if channels.len() > 32 {
        return Err(invalid());
    }
    channels
        .iter()
        .filter(|c| c.as_str() != ROOT)
        .map(|c| {
            store
                .ahp_access(user, c)
                .map_err(|_| Fault(-32001, "AHP session not found"))
        })
        .collect()
}
fn visibility(owned: &[crate::store::AhpAccess]) -> BTreeMap<String, String> {
    owned
        .iter()
        .map(|a| {
            (
                format!(
                    "{}{}",
                    if a.chat {
                        "ahp-chat:/"
                    } else {
                        "ahp-session:/"
                    },
                    a.id
                ),
                a.read_key.clone(),
            )
        })
        .collect()
}
fn pinned_accesses(
    conn: &Connection,
    store: &crate::store::Store,
    user: &str,
) -> Result<Vec<crate::store::AhpAccess>> {
    let owned = accesses(store, user, &conn.channels)?;
    if visibility(&owned) != conn.visibility {
        return Err(Fault(
            -32011,
            "Read authority changed; reconnect for a fresh snapshot",
        ));
    }
    Ok(owned)
}
fn subscriptions(owned: &[crate::store::AhpAccess]) -> Vec<ChannelSubscription<'_>> {
    owned
        .iter()
        .map(|s| ChannelSubscription {
            context: &s.context,
            session: &s.id,
            chat: s.chat,
        })
        .collect()
}
fn snapshots(
    store: &crate::store::Store,
    user: &str,
    channels: &BTreeSet<String>,
    device: &str,
    views: &BTreeMap<String, usize>,
) -> Result<(u64, Vec<Snapshot>, HashMap<String, HistoryWindow>)> {
    let owned = accesses(store, user, channels)?;
    let cut = store
        .interactive_journal()
        .channel_snapshots_with_views(&subscriptions(&owned), views)
        .map_err(|_| {
            Fault(
                -32602,
                "Snapshot exceeds its byte/turn budget; request a bounded chat view",
            )
        })?;
    let mut values = cut.snapshots;
    for snapshot in &mut values {
        if let Some(access) = owned.iter().find(|access| {
            snapshot.resource
                == format!(
                    "{}{}",
                    if access.chat {
                        "ahp-chat:/"
                    } else {
                        "ahp-session:/"
                    },
                    access.id
                )
        }) {
            let (_, _, owner) = store
                .interactive_identity(&access.id)
                .map_err(|_| unavailable())?;
            if owner != user {
                match &mut snapshot.state {
                    ahp_types::state::SnapshotState::Chat(chat) => {
                        chat.interactivity = Some(ahp_types::state::ChatInteractivity::ReadOnly);
                        shared_meta(&mut chat.meta, &owner);
                    }
                    ahp_types::state::SnapshotState::Session(session) => {
                        shared_meta(&mut session.meta, &owner);
                        for chat in &mut session.chats {
                            chat.interactivity =
                                Some(ahp_types::state::ChatInteractivity::ReadOnly);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let mut history = HashMap::new();
    for entry in cut.histories {
        let channel = entry.history.resource.clone();
        let access = owned
            .iter()
            .find(|a| a.chat && channel == format!("ahp-chat:/{}", a.id))
            .ok_or_else(unavailable)?;
        let token = uuid::Uuid::now_v7().to_string();
        let snapshot = values
            .iter_mut()
            .find(|s| s.resource == channel)
            .ok_or_else(unavailable)?;
        if let ahp_types::state::SnapshotState::Chat(chat) = &mut snapshot.state {
            chat.turns_next_cursor = Some(token.clone());
        } else {
            return Err(unavailable());
        }
        history.insert(
            channel,
            HistoryWindow {
                frozen: entry.history,
                before: entry.before,
                token,
                user: user.into(),
                device: device.into(),
                read_key: access.read_key.clone(),
                created: Instant::now(),
            },
        );
    }
    history_budget(&history)?;
    if channels.contains(ROOT) {
        values.insert(0, root(cut.cursor.sequence)?);
    }
    Ok((cut.cursor.sequence, values, history))
}
fn collect_summaries(
    state: &AppState,
    store: &crate::store::Store,
    user: &str,
) -> Result<BTreeMap<String, SessionSummary>> {
    Ok(store
        .ahp_summaries(user)
        .map_err(|_| unavailable())?
        .into_iter()
        .map(|mut s| {
            let shared = s
                .meta
                .as_ref()
                .and_then(|meta| meta.get("dunlin.ownerId"))
                .and_then(Value::as_str)
                .is_some_and(|owner| owner != user);
            if shared && let Some(chats) = &mut s.chats {
                for chat in chats {
                    chat.interactivity = Some(ahp_types::state::ChatInteractivity::ReadOnly);
                }
            }
            let local_enabled = !shared
                && s.resource
                    .strip_prefix("ahp-session:/")
                    .is_some_and(|id| state.interactive.queue_text_allowed(state, store, id, user));
            let remote_enabled = !shared
                && s.resource.strip_prefix("ahp-session:/").is_some_and(|id| {
                    state
                        .remote_sessions
                        .get()
                        .is_some_and(|service| service.queue_allowed(state, store, id, user))
                });
            let remote_stopped = s
                .resource
                .strip_prefix("ahp-session:/")
                .and_then(|id| store.remote_session_stop_observation(id).ok().flatten());
            let enabled = local_enabled || remote_enabled;
            s.meta
                .get_or_insert_with(Default::default)
                .insert("dunlin.queueText".into(), json!(enabled));
            s.meta
                .as_mut()
                .unwrap()
                .insert("dunlin.controlTurns".into(), json!(enabled));
            s.meta.as_mut().unwrap().insert(
                "dunlin.approveTools".into(),
                json!(
                    local_enabled
                        || !shared
                            && s.resource
                                .strip_prefix("ahp-session:/")
                                .is_some_and(|id| state.remote_sessions.get().is_some_and(
                                    |service| service.approvals_allowed(state, store, id, user)
                                ))
                ),
            );
            s.meta.as_mut().unwrap().insert(
                "dunlin.cancelClosesSession".into(),
                json!(remote_stopped.is_some()),
            );
            if let Some(stopped) = remote_stopped {
                s.meta
                    .as_mut()
                    .unwrap()
                    .insert("dunlin.remoteStopped".into(), json!(stopped));
            }
            s.meta
                .as_mut()
                .unwrap()
                .insert("dunlin.observerOnly".into(), json!(shared));
            (s.resource.clone(), s)
        })
        .collect())
}
fn root_updates(conn: &mut Connection, current: BTreeMap<String, SessionSummary>) -> Vec<Value> {
    let mut out = Vec::new();
    if conn.channels.contains(ROOT) {
        for (uri, summary) in &current {
            if let Some(previous) = conn.summaries.get(uri) {
                if previous != summary {
                    let mut changes = serde_json::to_value(summary).unwrap_or(Value::Null);
                    if let Some(object) = changes.as_object_mut() {
                        for key in ["resource", "provider", "createdAt"] {
                            object.remove(key);
                        }
                    }
                    out.push(json!({"jsonrpc":"2.0","method":"root/sessionSummaryChanged","params":{"channel":ROOT,"session":uri,"changes":changes}}));
                }
            } else {
                out.push(json!({"jsonrpc":"2.0","method":"root/sessionAdded","params":{"channel":ROOT,"summary":summary}}));
            }
        }
        for uri in conn.summaries.keys() {
            if !current.contains_key(uri) {
                out.push(json!({"jsonrpc":"2.0","method":"root/sessionRemoved","params":{"channel":ROOT,"session":uri}}));
            }
        }
    }
    conn.summaries = current;
    out
}
fn refresh(
    conn: &mut Connection,
    state: &AppState,
    store: &crate::store::Store,
    user: &str,
) -> Result<Vec<Value>> {
    let owned = pinned_accesses(conn, store, user)?;
    let replay = store
        .interactive_journal()
        .channel_replay(&subscriptions(&owned), conn.cursor)
        .map_err(|_| unavailable())?;
    let mut out = match replay {
        ChannelReplay::Actions {
            cursor,
            mut actions,
        } => {
            reader_actions(store, user, &mut actions)?;
            conn.cursor = cursor.sequence;
            action_messages(actions)
        }
        ChannelReplay::SnapshotRequired => {
            return Err(Fault(-32011, "Reconnect for a fresh snapshot"));
        }
    };
    out.extend(root_updates(conn, collect_summaries(state, store, user)?));
    Ok(out)
}
fn channels(values: Vec<String>) -> Result<BTreeSet<String>> {
    if values.len() > 32 || values.iter().any(|v| v.len() > 256) {
        return Err(invalid());
    }
    let count = values.len();
    let unique = BTreeSet::from_iter(values);
    if unique.len() != count {
        return Err(invalid());
    }
    Ok(unique)
}
fn supported_dispatch_shape(p: &Value) -> bool {
    fn keys(v: &Value, allowed: &[&str]) -> bool {
        v.as_object()
            .is_some_and(|m| m.keys().all(|k| allowed.contains(&k.as_str())))
    }
    if !keys(p, &["channel", "clientSeq", "action"]) {
        return false;
    }
    let a = &p["action"];
    match a["type"].as_str() {
        Some("chat/pendingMessageSet") => {
            keys(a, &["type", "kind", "id", "message"])
                && a["kind"] == "queued"
                && keys(&a["message"], &["text", "origin"])
                && keys(&a["message"]["origin"], &["kind"])
                && a["message"]["origin"]["kind"] == "user"
        }
        Some("chat/pendingMessageRemoved") => {
            keys(a, &["type", "kind", "id"]) && a["kind"] == "queued"
        }
        Some("chat/queuedMessagesReordered") => keys(a, &["type", "order"]),
        Some("chat/toolCallConfirmed") => keys(
            a,
            &[
                "type",
                "turnId",
                "toolCallId",
                "approved",
                "selectedOptionId",
            ],
        ),
        Some("chat/inputCompleted") => {
            keys(a, &["type", "requestId", "response", "answers"])
                && a["requestId"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("dunlin:mcp-input:"))
                && a.get("answers").is_none_or(|answers| {
                    answers.as_object().is_some_and(|answers| {
                        answers.len() <= 32
                            && answers.values().all(|answer| {
                                keys(answer, &["state", "value"])
                                    && answer["state"] == "submitted"
                                    && keys(&answer["value"], &["kind", "value"])
                            })
                    })
                })
        }
        Some("chat/turnCancelled") => keys(a, &["type", "turnId", "duration"]),
        _ => false,
    }
}

fn handle(
    conn: &mut Connection,
    state: &AppState,
    token: &str,
    method: &str,
    params: Value,
) -> Result<Reply> {
    conn.history
        .retain(|_, window| window.created.elapsed() <= HISTORY_TTL);
    let mut staged = conn.clone();
    let reply = match handle_inner(&mut staged, state, token, method, params) {
        Ok(reply) => reply,
        Err(error) => {
            if guard(state, token, conn).is_err() {
                conn.history.clear();
                conn.history_finished.clear();
            }
            return Err(error);
        }
    };
    *conn = staged;
    Ok(reply)
}
fn handle_inner(
    conn: &mut Connection,
    state: &AppState,
    token: &str,
    method: &str,
    params: Value,
) -> Result<Reply> {
    let store = state.store.lock().map_err(|_| unavailable())?;
    let auth = authorized_in_store(state, &store, token)
        .map_err(|_| Fault(-32009, "Authentication required"))?;
    let mut before = Vec::new();
    let result = match method {
        "ping" => {
            if params["channel"] != ROOT {
                return Err(invalid());
            }
            Value::Null
        }
        "initialize" => {
            if conn.client.is_some() {
                return Err(Fault(-32600, "Connection is already initialized"));
            }
            let p: InitializeParams = serde_json::from_value(params).map_err(|_| invalid())?;
            if p.channel != ROOT || !valid_client(&p.client_id) || p.protocol_versions.len() > 16 {
                return Err(invalid());
            }
            let version = ahp_types::version::negotiate_protocol_version(&p.protocol_versions)
                .map_err(|_| invalid())?
                .ok_or(Fault(-32005, "Unsupported AHP protocol version"))?
                .to_owned();
            let selected = channels(p.initial_subscriptions.unwrap_or_default())?;
            let (seq, snaps, history) = snapshots(
                &store,
                &auth.user.id,
                &selected,
                &auth.device.id,
                &BTreeMap::new(),
            )?;
            let pins = visibility(&accesses(&store, &auth.user.id, &selected)?);
            store
                .ahp_register_client(
                    &auth.device.id,
                    &p.client_id,
                    &version,
                    &state.browser_sessions.boot,
                    &pins,
                )
                .map_err(|_| unavailable())?;
            conn.client = Some(p.client_id);
            conn.channels = selected;
            conn.visibility = pins;
            conn.views.clear();
            conn.history = history;
            conn.history_finished.clear();
            store
                .ahp_set_views(
                    &auth.device.id,
                    conn.client.as_deref().ok_or_else(invalid)?,
                    &conn.views,
                )
                .map_err(|_| unavailable())?;
            conn.cursor = seq;
            conn.summaries = collect_summaries(state, &store, &auth.user.id)?;
            json!({"protocolVersion":version,"serverSeq":seq,"snapshots":snaps,"serverInfo":{"name":"Dunlin","version":env!("CARGO_PKG_VERSION")},"_meta":{"dunlin.observerOnly":false,"dunlin.queueText":true,"dunlin.controlTurns":true,"dunlin.approveTools":true}})
        }
        "reconnect" => {
            if conn.client.is_some() {
                return Err(Fault(-32600, "Connection is already initialized"));
            }
            let p: ReconnectParams = serde_json::from_value(params).map_err(|_| invalid())?;
            if p.channel != ROOT || !valid_client(&p.client_id) || p.last_seen_server_seq < 0 {
                return Err(invalid());
            }
            let previous = store
                .ahp_client(&auth.device.id, &p.client_id)
                .map_err(|_| unavailable())?
                .ok_or(Fault(-32600, "Initialize this client first"))?;
            let requested = channels(p.subscriptions)?;
            let mut selected = BTreeSet::new();
            let mut missing = Vec::new();
            for c in requested {
                if c == ROOT || store.ahp_access(&auth.user.id, &c).is_ok() {
                    selected.insert(c);
                } else {
                    missing.push(c);
                }
            }
            let owned = accesses(&store, &auth.user.id, &selected)?;
            let pins = visibility(&owned);
            let views: BTreeMap<_, _> = previous
                .views
                .iter()
                .filter(|(channel, _)| selected.contains(*channel))
                .map(|(c, n)| (c.clone(), *n))
                .collect();
            let same_scope = pins
                .iter()
                .all(|(channel, key)| previous.visibility.get(channel) == Some(key));
            let replay =
                if previous.boot == state.browser_sessions.boot && same_scope && views.is_empty() {
                    store
                        .interactive_journal()
                        .channel_replay(&subscriptions(&owned), p.last_seen_server_seq as u64)
                        .map_err(|_| unavailable())?
                } else {
                    ChannelReplay::SnapshotRequired
                };
            let result = match replay {
                ChannelReplay::Actions {
                    cursor,
                    mut actions,
                } => {
                    reader_actions(&store, &auth.user.id, &mut actions)?;
                    conn.cursor = cursor.sequence;
                    json!({"type":"replay","actions":actions,"missing":missing})
                }
                ChannelReplay::SnapshotRequired => {
                    let (seq, snaps, history) =
                        snapshots(&store, &auth.user.id, &selected, &auth.device.id, &views)?;
                    conn.cursor = seq;
                    conn.history = history;
                    conn.history_finished = views
                        .keys()
                        .filter(|channel| !conn.history.contains_key(*channel))
                        .cloned()
                        .collect();
                    json!({"type":"snapshot","snapshots":snaps})
                }
            };
            store
                .ahp_register_client(
                    &auth.device.id,
                    &p.client_id,
                    &previous.version,
                    &state.browser_sessions.boot,
                    &pins,
                )
                .map_err(|_| unavailable())?;
            conn.client = Some(p.client_id);
            conn.channels = selected;
            conn.visibility = pins;
            conn.views = views;
            store
                .ahp_set_views(
                    &auth.device.id,
                    conn.client.as_deref().ok_or_else(invalid)?,
                    &conn.views,
                )
                .map_err(|_| unavailable())?;
            conn.summaries = collect_summaries(state, &store, &auth.user.id)?;
            result
        }
        _ if conn.client.is_none() => {
            return Err(Fault(-32600, "Initialize this connection first"));
        }
        "dispatchAction" => {
            let strict = supported_dispatch_shape(&params);
            let p: DispatchActionParams = serde_json::from_value(params).map_err(|_| invalid())?;
            let client = conn.client.as_ref().ok_or_else(invalid)?;
            if !conn.channels.contains(&p.channel)
                || !(0..=9_007_199_254_740_991).contains(&p.client_seq)
            {
                return Err(invalid());
            }
            let access = store
                .ahp_access(&auth.user.id, &p.channel)
                .map_err(|_| Fault(-32009, "Dispatch authority denied"))?;
            if !access.chat {
                return Err(invalid());
            }
            let (workspace, project) = store
                .interactive_scope(&access.id)
                .map_err(|_| unavailable())?;
            access
                .context
                .authorize_project(
                    &workspace,
                    &project,
                    dunlin_domain::collaboration::Permission::CreateSession,
                )
                .map_err(|_| Fault(-32009, "Dispatch authority denied"))?;
            let local_allowed =
                state
                    .interactive
                    .queue_text_allowed(state, &store, &access.id, &auth.user.id);
            let remote_allowed = state.remote_sessions.get().is_some_and(|service| {
                service.queue_allowed(state, &store, &access.id, &auth.user.id)
            });
            let rejection = if !strict {
                Some("Unsupported chat control or message fields")
            } else if !local_allowed && !remote_allowed {
                Some("The session runtime is unavailable")
            } else if remote_allowed
                && !state.remote_sessions.get().is_some_and(|service| {
                    service.approvals_allowed(state, &store, &access.id, &auth.user.id)
                })
                && matches!(
                    &p.action,
                    ahp_types::actions::StateAction::ChatToolCallConfirmed(_)
                        | ahp_types::actions::StateAction::ChatInputCompleted(_)
                )
            {
                Some("Remote tool approvals are unavailable")
            } else if remote_allowed
                && matches!(&p.action,ahp_types::actions::StateAction::ChatPendingMessageSet(action) if action.message.text.len()>dunlin_relay_protocol::worker::session::MAX_SESSION_INPUT_BYTES)
            {
                Some("Remote prompt exceeds the session input limit")
            } else {
                None
            };
            let envelope = store
                .interactive_journal()
                .dispatch_action_with_mcp_input(
                    &access.context,
                    &access.id,
                    &auth.device.id,
                    client,
                    &p,
                    dunlin_session_runtime::DispatchAdmission {
                        rejection,
                        elapsed_ms: match &p.action {
                            ahp_types::actions::StateAction::ChatTurnCancelled(a) => {
                                state.interactive.turn_elapsed(&access.id, &a.turn_id)
                            }
                            _ => None,
                        },
                    },
                    &crate::interactive::mcp::forms::validate,
                )
                .map_err(|e| match e {
                    dunlin_session_runtime::Error::Policy(_) => {
                        Fault(-32009, "Dispatch authority denied")
                    }
                    dunlin_session_runtime::Error::Conflict(_)
                    | dunlin_session_runtime::Error::Limit(_) => {
                        Fault(-32602, "Dispatch cannot be reconciled automatically")
                    }
                    _ => unavailable(),
                })?;
            if envelope.rejection_reason.is_none()
                && let ahp_types::actions::StateAction::ChatTurnCancelled(action) = &p.action
                && let Some(service) = state.remote_sessions.get()
            {
                service.signal_cancellation(
                    &store,
                    &access.id,
                    &auth.user.id,
                    &access.context,
                    &action.turn_id,
                );
            }
            before = refresh(conn, state, &store, &auth.user.id)?;
            // A retry may be older than a snapshot. Echo its durable outcome even
            // when it is outside replay; clients reconcile origin before dedupe.
            if !before
                .iter()
                .any(|v| v["method"] == "action" && v["params"]["serverSeq"] == envelope.server_seq)
            {
                before.extend(action_messages(vec![envelope]));
            }
            Value::Null
        }
        "subscribe" => {
            let p: SubscribeParams = serde_json::from_value(params).map_err(|_| invalid())?;
            let mut selected = conn.channels.clone();
            selected.insert(p.channel.clone());
            if selected.len() > 32 {
                return Err(invalid());
            }
            // Catch up existing subscriptions and snapshot the new channel while
            // holding the same Store lock, so advancing the cut cannot skip data.
            let mut pending = conn.clone();
            before = refresh(&mut pending, state, &store, &auth.user.id)?;
            if let Some(turns) = p.view.as_ref().and_then(|view| view.turns) {
                if !(0..=64).contains(&turns) || !p.channel.starts_with("ahp-chat:/") {
                    return Err(invalid());
                }
                pending.views.insert(p.channel.clone(), turns as usize);
            } else {
                pending.views.remove(&p.channel);
            }
            pending.history.remove(&p.channel);
            pending.history_finished.remove(&p.channel);
            let (seq, mut snaps, history) = snapshots(
                &store,
                &auth.user.id,
                &BTreeSet::from([p.channel.clone()]),
                &auth.device.id,
                &pending.views,
            )?;
            for (channel, window) in history {
                let bytes = pending
                    .history
                    .values()
                    .map(|h| h.frozen.bytes)
                    .sum::<usize>();
                let turns = pending
                    .history
                    .values()
                    .map(|h| h.frozen.turn_count())
                    .sum::<usize>();
                if bytes
                    .checked_add(window.frozen.bytes)
                    .is_none_or(|value| value > dunlin_session_runtime::HISTORY_BYTES)
                    || turns
                        .checked_add(window.frozen.turn_count())
                        .is_none_or(|value| value > dunlin_session_runtime::HISTORY_TURNS)
                {
                    return Err(Fault(
                        -32602,
                        "Connection history budget exceeded; unsubscribe another chat",
                    ));
                }
                pending.history.insert(channel, window);
            }
            if pending.views.contains_key(&p.channel) && !pending.history.contains_key(&p.channel) {
                pending.history_finished.insert(p.channel.clone());
            }
            history_budget(&pending.history)?;
            let snapshot = snaps.pop().ok_or_else(unavailable)?;
            pending.visibility = visibility(&accesses(&store, &auth.user.id, &selected)?);
            store
                .ahp_set_visibility(
                    &auth.device.id,
                    pending.client.as_deref().ok_or_else(invalid)?,
                    &pending.visibility,
                )
                .map_err(|_| unavailable())?;
            pending.channels = selected;
            pending.cursor = seq;
            store
                .ahp_set_views(
                    &auth.device.id,
                    pending.client.as_deref().ok_or_else(invalid)?,
                    &pending.views,
                )
                .map_err(|_| unavailable())?;
            *conn = pending;
            json!({"snapshot":snapshot})
        }
        "fetchTurns" => {
            let p: FetchTurnsParams = serde_json::from_value(params).map_err(|_| invalid())?;
            if !conn.channels.contains(&p.channel) {
                return Err(invalid());
            }
            // Authorization/visibility is rechecked before even inspecting an
            // opaque cursor. A revoke/regrant cannot make an old frozen cut live.
            let owned = pinned_accesses(conn, &store, &auth.user.id)?;
            let access = owned
                .iter()
                .find(|a| a.chat && p.channel == format!("ahp-chat:/{}", a.id))
                .ok_or_else(invalid)?;
            before = refresh(conn, state, &store, &auth.user.id)?;
            let Some(window) = conn.history.get(&p.channel).cloned() else {
                if p.cursor.is_some()
                    || (conn.views.contains_key(&p.channel)
                        && !conn.history_finished.contains(&p.channel))
                {
                    return Err(invalid());
                }
                return Ok(Reply {
                    result: json!({}),
                    before,
                });
            };
            if window.created.elapsed() > HISTORY_TTL
                || window.user != auth.user.id
                || window.device != auth.device.id
                || window.read_key != access.read_key
                || p.cursor
                    .as_ref()
                    .is_some_and(|cursor| cursor != &window.token)
            {
                return Err(invalid());
            }
            let journal = store.interactive_journal();
            if journal.channel_clock().map_err(|_| unavailable())?.stream
                != window.frozen.cut.stream
            {
                return Err(invalid());
            }
            let page = window
                .frozen
                .page(window.before)
                .map_err(|_| Fault(-32602, "Historical turn exceeds the history page budget"))?;
            let next = page.next_before.map(|_| uuid::Uuid::now_v7().to_string());
            let action = serde_json::from_value::<ahp_types::actions::StateAction>(
                json!({"type":"chat/turnsLoaded","turns":page.turns,"turnsNextCursor":next}),
            )
            .map_err(|_| unavailable())?;
            // Everything above is fallible without moving the global sequence.
            // The reserved slot is requester-only; no canonical history payload,
            // origin or connection cursor is written to the shared action log.
            let seq = journal
                .reserve_history_sequence(&access.context, &access.id)
                .map_err(|_| unavailable())?;
            let envelope = ahp_types::actions::ActionEnvelope {
                server_seq: seq,
                channel: p.channel.clone(),
                action,
                origin: None,
                rejection_reason: None,
            };
            before.extend(action_messages(vec![envelope]));
            conn.cursor = seq;
            if let Some(before) = page.next_before {
                let mut window = window;
                window.before = before;
                window.token = next.ok_or_else(unavailable)?;
                conn.history.insert(p.channel, window);
            } else {
                conn.history.remove(&p.channel);
                conn.history_finished.insert(p.channel);
            }
            json!({})
        }
        "unsubscribe" => {
            let channel = params["channel"].as_str().ok_or_else(invalid)?;
            conn.channels.remove(channel);
            conn.visibility.remove(channel);
            conn.views.remove(channel);
            conn.history.remove(channel);
            conn.history_finished.remove(channel);
            store
                .ahp_set_visibility(
                    &auth.device.id,
                    conn.client.as_deref().ok_or_else(invalid)?,
                    &conn.visibility,
                )
                .map_err(|_| unavailable())?;
            store
                .ahp_set_views(
                    &auth.device.id,
                    conn.client.as_deref().ok_or_else(invalid)?,
                    &conn.views,
                )
                .map_err(|_| unavailable())?;
            Value::Null
        }
        "listSessions" => {
            let p: ListSessionsParams = serde_json::from_value(params).map_err(|_| invalid())?;
            if p.channel != ROOT {
                return Err(invalid());
            }
            let limit = p.limit.unwrap_or(32);
            if !(1..=64).contains(&limit) {
                return Err(invalid());
            }
            let mut current = collect_summaries(state, &store, &auth.user.id)?
                .into_values()
                .collect::<Vec<_>>();
            current.sort_by(|a, b| {
                b.modified_at
                    .cmp(&a.modified_at)
                    .then_with(|| a.resource.cmp(&b.resource))
            });
            let ids = if let Some(cursor) = p.cursor {
                conn.pages.get(&cursor).cloned().ok_or_else(invalid)?
            } else {
                current.iter().map(|s| s.resource.clone()).collect()
            };
            let by_id: BTreeMap<_, _> = current
                .into_iter()
                .map(|s| (s.resource.clone(), s))
                .collect();
            let visible = ids
                .into_iter()
                .filter_map(|id| by_id.get(&id).cloned())
                .collect::<Vec<_>>();
            let items = visible
                .iter()
                .take(limit as usize)
                .cloned()
                .collect::<Vec<_>>();
            let next = if visible.len() > limit as usize {
                let token = uuid::Uuid::now_v7().to_string();
                if conn.pages.len() >= 8 {
                    conn.pages.clear();
                }
                conn.pages.insert(
                    token.clone(),
                    visible
                        .iter()
                        .skip(limit as usize)
                        .map(|s| s.resource.clone())
                        .collect(),
                );
                Some(token)
            } else {
                None
            };
            serde_json::to_value(ahp_types::commands::ListSessionsResult {
                items,
                next_cursor: next,
            })
            .map_err(|_| unavailable())?
        }
        _ => {
            return Err(Fault(
                -32601,
                "Operation is unavailable on the observer transport",
            ));
        }
    };
    Ok(Reply { result, before })
}
pub(in crate::api) async fn upgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> std::result::Result<Response, ApiError> {
    request_origin(&headers, true)?;
    let token = cookie_token(&headers)?;
    authorized(&state, &token)?;
    if (!state.config.experimental_interactive_runtime && state.remote_sessions.get().is_none())
        || !state.config.store_transcripts
    {
        return Err(ApiError::not_found());
    }
    let permit = state
        .browser_sessions
        .connections
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Live connection limit reached",
            )
        })?;
    Ok(ws
        .max_message_size(64 * 1024)
        .max_frame_size(64 * 1024)
        .write_buffer_size(0)
        .max_write_buffer_size(FRAME)
        .on_upgrade(move |socket| live(socket, state, token, permit))
        .into_response())
}
fn guard(state: &AppState, token: &str, conn: &Connection) -> Result<()> {
    let store = state.store.lock().map_err(|_| unavailable())?;
    let auth = authorized_in_store(state, &store, token)
        .map_err(|_| Fault(-32009, "Authentication required"))?;
    pinned_accesses(conn, &store, &auth.user.id)?;
    Ok(())
}
async fn send(
    socket: &mut WebSocket,
    state: &AppState,
    token: &str,
    conn: &Connection,
    value: Value,
) -> Result<()> {
    guard(state, token, conn)?;
    let text = serde_json::to_string(&value).map_err(|_| unavailable())?;
    if text.len() > FRAME {
        return Err(Fault(-32011, "Reconnect for a fresh snapshot"));
    }
    tokio::time::timeout(
        Duration::from_secs(2),
        socket.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| unavailable())?
    .map_err(|_| unavailable())
}
async fn live(
    mut socket: WebSocket,
    state: AppState,
    token: String,
    _permit: OwnedSemaphorePermit,
) {
    // Registered before any initialization/snapshot cut, independent of actor receivers.
    let mut changes = match state.store.lock() {
        Ok(s) => s.subscribe_interactive(),
        Err(_) => return,
    };
    let mut conn = Connection::default();
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut pong = Instant::now();
    let mut ping = Instant::now();
    let mut window = Instant::now();
    let mut commands = 0usize;
    loop {
        let mut fault = None;
        tokio::select! {
            incoming=socket.recv()=>match incoming {
                Some(Ok(Message::Text(text)))=>{
                    if window.elapsed()>Duration::from_secs(60){window=Instant::now();commands=0;}commands+=1;if commands>128{break;}
                    let value=serde_json::from_str::<Value>(&text);
                    match value {
                        Ok(v) if v.is_object() && v["jsonrpc"]=="2.0" && v["method"].is_string()=>{
                            let id=v.get("id").cloned();let method=v["method"].as_str().unwrap_or("");
                            if id.as_ref().is_some_and(|id|!(id.is_null()||id.is_number()||id.as_str().is_some_and(|s|s.len()<=128))){break;}
                            if id.is_some() && method=="dispatchAction" {
                                if send(&mut socket,&state,&token,&conn,json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"dispatchAction must be a notification"}})).await.is_err(){fault=Some(unavailable());}
                            } else if id.is_none() && !matches!(method,"unsubscribe"|"dispatchAction") {
                                fault=Some(invalid());
                            } else {
                            match handle(&mut conn,&state,&token,method,v.get("params").cloned().unwrap_or(Value::Null)) {
                                Ok(reply)=>{
                                    for value in reply.before{if let Err(e)=send(&mut socket,&state,&token,&conn,value).await{fault=Some(e);break;}}
                                    if fault.is_none() && let Some(id)=id && let Err(e)=send(&mut socket,&state,&token,&conn,json!({"jsonrpc":"2.0","id":id,"result":reply.result})).await {fault=Some(e);}
                                },
                                Err(error)=>{
                                    if let Some(id)=id.as_ref() {let mut body=json!({"code":error.0,"message":error.1});if error.0==-32005{body["data"]=json!({"supportedVersions":ahp_types::version::SUPPORTED_PROTOCOL_VERSIONS});}
                                        if send(&mut socket,&state,&token,&conn,json!({"jsonrpc":"2.0","id":id,"error":body})).await.is_err(){fault=Some(unavailable());}}
                                    if id.is_none()||error.0==-32005||error.0==-32011||error.0==-32009{fault=Some(error);}
                                },
                            }
                            }
                        },
                        Err(_)=>{let _=send(&mut socket,&state,&token,&conn,json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Invalid JSON"}})).await;break;},
                        _=>{let _=send(&mut socket,&state,&token,&conn,json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid JSON-RPC message"}})).await;break;},
                    }
                },
                Some(Ok(Message::Pong(_)))=>pong=Instant::now(),
                Some(Ok(Message::Ping(_)))=>{},
                _=>break,
            },
            changed=changes.recv()=>{
                if conn.client.is_some(){match changed {
                    Ok(())=>{
                        let values=(||{let store=state.store.lock().map_err(|_|unavailable())?;let auth=authorized_in_store(&state,&store,&token).map_err(|_|Fault(-32009,"Authentication required"))?;refresh(&mut conn,&state,&store,&auth.user.id)})();
                        match values{Ok(values)=>for v in values{if let Err(e)=send(&mut socket,&state,&token,&conn,v).await{fault=Some(e);break;}},Err(e)=>fault=Some(e)}
                    },
                    Err(_)=>fault=Some(Fault(-32011,"Reconnect for a fresh snapshot")),
                }}
            },
            _=tick.tick()=>{
                conn.history.retain(|_,window|window.created.elapsed()<=HISTORY_TTL);
                if let Err(e)=guard(&state,&token,&conn){fault=Some(e);}
                if pong.elapsed()>Duration::from_secs(45){break;}
                if ping.elapsed()>Duration::from_secs(15){ping=Instant::now();if tokio::time::timeout(Duration::from_secs(2),socket.send(Message::Ping(Vec::new().into()))).await.is_err(){break;}}
            },
        }
        if let Some(error) = fault {
            let _ = tokio::time::timeout(
                Duration::from_secs(1),
                socket.send(Message::Close(Some(CloseFrame {
                    code: if error.0 == -32011 { 1013 } else { 1008 },
                    reason: error.1.into(),
                }))),
            )
            .await;
            break;
        }
    }
}
#[cfg(test)]
mod tests;
