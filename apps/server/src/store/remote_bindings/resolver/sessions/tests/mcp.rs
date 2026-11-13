use super::*;
use crate::{api::AppState, config::Config, interactive::mcp::Selection};
use dunlin_mcp_client::oauth::{AuthContext, persistence::CredentialJournal};
use std::sync::Arc;
mod authority;
mod bridge;
mod remote;
mod runtime;

struct McpFixture {
    data: Fixture,
    state: AppState,
    context: AuthContext,
    transport: crate::worker_channel::SessionTransport,
    _registration: Box<dyn Send>,
    _producer: Option<crate::worker_channel::SessionProducerRegistration>,
    _events: tokio::sync::mpsc::Receiver<crate::worker_channel::SessionEvent>,
}
impl McpFixture {
    fn new(store: Store) -> Self {
        let mut data = Fixture::with_store(store);
        let store = std::mem::replace(&mut data.store, Store::open_in_memory().unwrap());
        let provider = serde_json::from_value(json!({"id":"work","name":"Work","endpoint":"https://mcp.example.test/mcp","issuer":"https://auth.example.test","client_id":"dunlin","redirect_uri":"https://relay.tail123.ts.net/v1/web/mcp/oauth/work/callback","client_secret_ref":null})).unwrap();
        let config = Config {
            mcp_oauth: vec![provider],
            worker_controller: Some(crate::worker_channel::ControllerConfig {
                execution_enabled: true,
                persistent_sessions: true,
                mcp_broker: true,
                socket: "/fixture/broker.sock".into(),
                node_id: "controller-node".into(),
                serve_origin: "https://relay.tail123.ts.net".into(),
            }),
            ..Default::default()
        };
        let mut state = AppState::new(store, config);
        Arc::get_mut(&mut state.worker_channels).unwrap().boot_id = "controller-boot".into();
        let (sender, events) = tokio::sync::mpsc::channel(8);
        let producer = state
            .worker_channels
            .register_session_producer(sender)
            .unwrap();
        let (transport, registration) = state
            .worker_channels
            .register_broker_fixture(
                &state,
                &data.identity,
                &data.channel,
                "controller-node".into(),
                wire::broker::BrokerLimits {
                    max_pending: 2,
                    max_reply_bytes: 65536,
                },
            )
            .unwrap();
        let context = AuthContext {
            user_id: data.owner.clone(),
            workspace_id: data.workspace.clone(),
            connection_id: uuid::Uuid::now_v7().to_string(),
            connection_revision: 1,
        };
        {
            let store = state.store.lock().unwrap();
            store
                .mcp_oauth_prepare(&context, &"b".repeat(64), "work", None, &|_, _| Ok(()))
                .unwrap();
            let intent = store.begin_install(&context, &"b".repeat(64)).unwrap();
            store
                .prepare_write(&context, &"b".repeat(64), &intent, &"d".repeat(64))
                .unwrap();
            store
                .commit(&context, &"b".repeat(64), &intent, &"d".repeat(64))
                .unwrap();
            let mut definition = store
                .agent_version(
                    &data.owner,
                    &data.workspace,
                    &data.project,
                    &data.request.profile.agent_id,
                    data.request.profile.version,
                )
                .unwrap()
                .unwrap()
                .definition;
            definition.schema_version = 2;
            definition.mcp.push(dunlin_domain::agent::AgentMcpTool {
                provider_id: "work".into(),
                name: "lookup".into(),
                definition_digest: "a".repeat(64),
            });
            let draft = store
                .create_agent(&data.owner, &data.workspace, &data.project, definition)
                .unwrap();
            let profile = store
                .publish_agent(&data.owner, &data.workspace, &data.project, &draft.id, 1)
                .unwrap();
            data.request.profile = AgentProfileRef {
                agent_id: profile.agent_id,
                version: profile.version,
                digest: profile.digest,
            };
        }
        Self {
            data,
            state,
            context,
            transport,
            _registration: registration,
            _producer: Some(producer),
            _events: events,
        }
    }
    fn prepare(&self, store: &Store) -> Result<PreparedRemoteSession> {
        store.prepare_remote_mcp_session_in_transaction(
            &self.data.owner,
            &self.data.device,
            &self.data.workspace,
            &self.data.project,
            self.data.request.clone(),
            "controller-node",
            RemoteMcpAdmission {
                config: &self.state.config,
                selections: &[Selection {
                    provider_id: "work".into(),
                    connection_id: self.context.connection_id.clone(),
                    revision: self.context.connection_revision,
                }],
                transport: self.transport.clone(),
            },
        )
    }
    fn admit(
        &self,
        store: &Store,
        prepared: PreparedRemoteSession,
    ) -> Result<RemoteSessionAdmission> {
        store.admit_remote_session_in_transaction(
            prepared,
            self.data.proof(),
            self.data.fence(),
            "private-mcp-intent-0001",
            "mcp-runtime",
            now(),
            attest,
        )
    }
}

#[test]
fn private_mcp_admission_persists_pins_separately_and_legacy_dispatch_stays_closed() {
    let fixture = McpFixture::new(Store::open_in_memory().unwrap());
    let store = fixture.state.store.lock().unwrap();
    let admitted = store
        .remote_binding_transaction(false, |store| fixture.admit(store, fixture.prepare(store)?))
        .unwrap();
    assert!(admitted.admission.lease.is_some());
    let command = admitted.command.unwrap();
    let RemoteOpen::Broker(open) = &command else {
        panic!("v5 open required")
    };
    assert!(open.open.validate().is_err());
    let wire = serde_json::to_string(&command).unwrap();
    for forbidden in [
        fixture.context.connection_id.as_str(),
        "connections",
        "client_id",
        "access_token",
        "refresh_token",
        "secret_reference",
    ] {
        assert!(!wire.contains(forbidden));
    }
    let id = admitted.admission.session.id;
    let pins: String = store
        .connection
        .query_row(
            "SELECT pins FROM controller_remote_sessions WHERE session=?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(pins.contains(&fixture.context.connection_id));
    let snapshot = store.remote_session_snapshot(&id).unwrap();
    assert!(snapshot.mcp_binding.is_some());
    assert_eq!(snapshot.open, command);
    assert!(
        store
            .remote_binding_transaction(false, |store| store
                .dispatch_remote_session_open_in_transaction(
                    &id,
                    fixture.data.proof(),
                    now(),
                    source
                ))
            .is_err()
    );
    assert!(!store.remote_session_snapshot(&id).unwrap().dispatched);
    store.disable(&fixture.context).unwrap();
    assert!(
        store
            .remote_session_snapshot(&id)
            .unwrap()
            .mcp_binding
            .is_some()
    );
    assert_eq!(store.recover_remote_sessions().unwrap(), 1);
    let stopped = store.remote_session_snapshot(&id).unwrap();
    assert_eq!(stopped.phase, "stopped");
    assert!(!stopped.dispatched);
}

#[test]
fn legacy_open_and_pins_keep_their_original_json_representation() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let admitted = fixture
        .store
        .remote_binding_transaction(false, |store| {
            store.admit_remote_session_in_transaction(
                prepared,
                fixture.proof(),
                fixture.fence(),
                "legacy-shape-key-0001",
                "runtime",
                now(),
                attest,
            )
        })
        .unwrap();
    let id = admitted.admission.session.id;
    let command = admitted.command.unwrap();
    let RemoteOpen::Legacy(base) = &command else {
        panic!("legacy open required")
    };
    assert_eq!(
        serde_json::to_string(&command).unwrap(),
        serde_json::to_string(base).unwrap()
    );
    assert_eq!(
        command.fingerprint().unwrap(),
        valid(wire::session_open_fingerprint(base)).unwrap()
    );
    let pins: String = fixture
        .store
        .connection
        .query_row(
            "SELECT pins FROM controller_remote_sessions WHERE session=?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !serde_json::from_str::<serde_json::Value>(&pins)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("mcp")
    );
}

#[test]
fn private_mcp_retry_after_reopen_never_mints_a_second_lease_or_open_command() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let fixture = McpFixture::new(Store::open(&path).unwrap());
    let first = {
        let store = fixture.state.store.lock().unwrap();
        store
            .remote_binding_transaction(false, |store| {
                fixture.admit(store, fixture.prepare(store)?)
            })
            .unwrap()
    };
    let reopened = Store::open(&path).unwrap();
    let again = reopened
        .remote_binding_transaction(false, |store| fixture.admit(store, fixture.prepare(store)?))
        .unwrap();
    assert_eq!(first.admission.session.id, again.admission.session.id);
    assert!(again.admission.lease.is_none());
    assert!(again.command.is_none());
    assert_eq!(
        reopened
            .connection
            .query_row(
                "SELECT count(*) FROM controller_remote_sessions",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn private_mcp_prepare_rechecks_revocation_and_source_registration_before_commit() {
    for disconnect in [true, false] {
        let mut fixture = McpFixture::new(Store::open_in_memory().unwrap());
        let store = fixture.state.store.lock().unwrap();
        let prepared = store
            .remote_binding_transaction(false, |store| fixture.prepare(store))
            .unwrap();
        if disconnect {
            store.disable(&fixture.context).unwrap();
        } else {
            drop(fixture._producer.take());
        }
        let result =
            store.remote_binding_transaction(false, |store| fixture.admit(store, prepared));
        assert!(result.is_err());
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT count(*) FROM controller_remote_sessions",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}

#[test]
fn private_mcp_sql_failure_rolls_back_journal_and_retry_uses_same_intent() {
    let fixture = McpFixture::new(Store::open_in_memory().unwrap());
    let store = fixture.state.store.lock().unwrap();
    store.connection.execute_batch("CREATE TRIGGER reject_private_mcp BEFORE INSERT ON controller_remote_sessions BEGIN SELECT RAISE(ABORT,'fixture');END").unwrap();
    assert!(
        store
            .remote_binding_transaction(false, |store| fixture
                .admit(store, fixture.prepare(store)?))
            .is_err()
    );
    for table in [
        "interactive_sessions",
        "interactive_session_admissions",
        "interactive_session_profiles",
        "controller_remote_sessions",
    ] {
        assert_eq!(
            store
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    store
        .connection
        .execute_batch("DROP TRIGGER reject_private_mcp")
        .unwrap();
    let accepted = store
        .remote_binding_transaction(false, |store| fixture.admit(store, fixture.prepare(store)?))
        .unwrap();
    assert!(accepted.admission.lease.is_some());
    assert!(accepted.command.is_some());
    let mut request = fixture.data.request.clone();
    request.profile.digest = "e".repeat(64);
    assert!(
        store
            .remote_binding_transaction(false, |store| store.prepare_remote_session_in_transaction(
                &fixture.data.owner,
                &fixture.data.device,
                &fixture.data.workspace,
                &fixture.data.project,
                request,
                "controller-node"
            ))
            .is_err()
    );
}

#[test]
fn mcp_receipt_lookup_survives_transport_drop_without_recreating_authority() {
    let fixture = McpFixture::new(Store::open_in_memory().unwrap());
    let store = fixture.state.store.lock().unwrap();
    let admitted = store
        .remote_binding_transaction(false, |store| fixture.admit(store, fixture.prepare(store)?))
        .unwrap();
    let id = admitted.admission.session.id.clone();
    let selections = vec![Selection {
        provider_id: "work".into(),
        connection_id: fixture.context.connection_id.clone(),
        revision: 1,
    }];
    drop(fixture._registration);
    let lookup = |selections: &[Selection], request: &RemoteSessionRequest| {
        store.remote_binding_transaction(false, |store| {
            store.lookup_remote_session_request_in_transaction(
                &fixture.data.owner,
                &fixture.data.device,
                &fixture.data.workspace,
                &fixture.data.project,
                request,
                "controller-node",
                &fixture.state.config,
                selections,
                "private-mcp-intent-0001",
                |_, _, _| Ok(()),
            )
        })
    };
    let replay = lookup(&selections, &fixture.data.request).unwrap().unwrap();
    assert_eq!(replay.id, id);
    let response = serde_json::to_value(&replay).unwrap();
    assert!(response.get("runtime").is_none() && response.get("authority").is_none());
    assert!(lookup(&[], &fixture.data.request).is_err());
    let mut changed = selections.clone();
    changed[0].revision += 1;
    assert!(lookup(&changed, &fixture.data.request).is_err());
    let mut request = fixture.data.request.clone();
    request.profile.digest = "f".repeat(64);
    assert!(lookup(&selections, &request).is_err());
    store.disable(&fixture.context).unwrap();
    assert!(lookup(&selections, &fixture.data.request).is_err());
    let count: i64 = store
        .connection
        .query_row(
            "SELECT count(*) FROM controller_remote_sessions",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}
