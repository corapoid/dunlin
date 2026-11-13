use super::*;
use dunlin_mcp_client::oauth::persistence::CredentialJournal;

const BINDING: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const TOKEN_DIGEST: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    config: Config,
    context: AuthContext,
    definition: AgentDefinition,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("relay.db")).unwrap();
        let owner = store.create_user("binding-owner", "admin").unwrap();
        let workspace = store.create_workspace(&owner, "Private binding").unwrap();
        let context = AuthContext {
            user_id: owner.id,
            workspace_id: workspace.id,
            connection_id: uuid::Uuid::now_v7().to_string(),
            connection_revision: 1,
        };
        let provider: Provider = serde_json::from_value(json!({"id":"work","name":"Work","endpoint":"https://mcp.example.test/mcp","issuer":"https://auth.example.test","client_id":"dunlin","redirect_uri":"https://relay.tail123.ts.net/v1/web/mcp/oauth/work/callback","client_secret_ref":null})).unwrap();
        provider.validate().unwrap();
        let config = Config {
            mcp_oauth: vec![provider],
            ..Default::default()
        };
        let definition = serde_json::from_value(json!({"schema_version":2,"name":"Private bindings","description":"","harness":"opencode","model":"model","mode":"mode","instructions":"instructions","skills":[],"mcp":[{"provider_id":"work","name":"lookup","definition_digest":"a".repeat(64)}],"max_seconds":60,"max_output_bytes":65536})).unwrap();
        install(&store, &context, "work");
        Self {
            dir,
            store,
            config,
            context,
            definition,
        }
    }
    fn selections(&self) -> Vec<Selection> {
        vec![Selection {
            provider_id: "work".into(),
            connection_id: self.context.connection_id.clone(),
            revision: self.context.connection_revision,
        }]
    }
    fn capture(&self) -> BoundSession {
        BoundSession::capture(
            &self.config,
            &self.store,
            &self.context.user_id,
            &self.context.workspace_id,
            &self.definition,
            &self.selections(),
        )
        .unwrap()
    }
    fn restore(&self, encoded: &str) -> Result<BoundSession> {
        BoundSession::restore(
            &self.config,
            &self.store,
            &self.context.user_id,
            &self.context.workspace_id,
            &self.definition,
            encoded,
        )
    }
}
fn install(store: &Store, context: &AuthContext, provider: &str) {
    store
        .mcp_oauth_prepare(context, BINDING, provider, None, &|_, _| Ok(()))
        .unwrap();
    let intent = store.begin_install(context, BINDING).unwrap();
    store
        .prepare_write(context, BINDING, &intent, TOKEN_DIGEST)
        .unwrap();
    store
        .commit(context, BINDING, &intent, TOKEN_DIGEST)
        .unwrap();
}

#[test]
fn private_snapshot_roundtrips_through_storage_and_reopened_oauth_journal_without_secrets() {
    let fixture = Fixture::new();
    let bound = fixture.capture();
    let encoded = bound.encode().unwrap();
    for forbidden in [
        "endpoint",
        "issuer",
        "client_id",
        "client_secret_ref",
        "secret_reference",
        "access_token",
        "refresh_token",
        TOKEN_DIGEST,
        BINDING,
    ] {
        assert!(!encoded.contains(forbidden), "{forbidden}");
    }
    let old_identity = bound.connections[0].identity();
    assert_eq!(
        old_identity,
        json!({"provider_id":"work","connection_id":fixture.context.connection_id,"revision":1,"configuration":fixture.config.mcp_oauth[0].digest()})
    );
    let db =
        rusqlite::Connection::open(fixture.dir.path().join("private-session-fixture.db")).unwrap();
    db.execute_batch("CREATE TABLE snapshots(id TEXT PRIMARY KEY, body TEXT NOT NULL)")
        .unwrap();
    db.execute("INSERT INTO snapshots VALUES('session',?1)", [&encoded])
        .unwrap();
    drop(db);
    let reopened = Store::open(fixture.dir.path().join("relay.db")).unwrap();
    let db =
        rusqlite::Connection::open(fixture.dir.path().join("private-session-fixture.db")).unwrap();
    let saved: String = db
        .query_row("SELECT body FROM snapshots WHERE id='session'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let restored = BoundSession::restore(
        &fixture.config,
        &reopened,
        &fixture.context.user_id,
        &fixture.context.workspace_id,
        &fixture.definition,
        &saved,
    )
    .unwrap();
    assert_eq!(restored.encode().unwrap(), encoded);
    assert_eq!(restored.connections[0].identity(), old_identity);
}

#[test]
fn restored_data_requires_current_owner_workspace_profile_and_operator_configuration() {
    let fixture = Fixture::new();
    let encoded = fixture.capture().encode().unwrap();
    for changed in ["owner", "workspace", "profile", "provider"] {
        let mut owner = fixture.context.user_id.clone();
        let mut workspace = fixture.context.workspace_id.clone();
        let mut definition = fixture.definition.clone();
        let mut config = fixture.config.clone();
        match changed {
            "owner" => owner = "another-user".into(),
            "workspace" => workspace = "another-workspace".into(),
            "profile" => definition.instructions = Some("changed instructions".into()),
            "provider" => config.mcp_oauth[0].endpoint = "https://other.example.test/mcp".into(),
            _ => unreachable!(),
        }
        assert!(
            BoundSession::restore(
                &config,
                &fixture.store,
                &owner,
                &workspace,
                &definition,
                &encoded
            )
            .is_err(),
            "{changed}"
        );
    }
    let mut value: Value = serde_json::from_str(&encoded).unwrap();
    value["owner"] = json!("another-user");
    // Editing the stored scope does not let that user borrow the original account.
    assert!(
        BoundSession::restore(
            &fixture.config,
            &fixture.store,
            "another-user",
            &fixture.context.workspace_id,
            &fixture.definition,
            &value.to_string()
        )
        .is_err()
    );
}

#[test]
fn disconnect_reauthorize_and_uncertain_refresh_never_resurrect_old_snapshot() {
    let mut fixture = Fixture::new();
    let encoded = fixture.capture().encode().unwrap();
    fixture.context.connection_revision = fixture.store.disable(&fixture.context).unwrap();
    assert!(fixture.restore(&encoded).is_err());
    install(&fixture.store, &fixture.context, "work");
    assert!(fixture.restore(&encoded).is_err());
    let fresh = fixture.capture().encode().unwrap();
    assert_ne!(fresh, encoded);
    let active = fixture.store.active(&fixture.context, BINDING).unwrap();
    fixture
        .store
        .begin_refresh(&fixture.context, BINDING, &active)
        .unwrap();
    assert!(fixture.restore(&fresh).is_err());
}

#[test]
fn malformed_extra_secret_duplicate_and_oversized_snapshot_data_is_rejected() {
    let fixture = Fixture::new();
    let encoded = fixture.capture().encode().unwrap();
    for changed in [
        "version",
        "digest",
        "configuration",
        "revision",
        "duplicate",
        "token",
        "nested-token",
    ] {
        let mut value: Value = serde_json::from_str(&encoded).unwrap();
        match changed {
            "version" => value["schema_version"] = json!(2),
            "digest" => value["profile_digest"] = json!("not-a-digest"),
            "configuration" => value["connections"][0]["configuration"] = json!("e".repeat(64)),
            "revision" => value["connections"][0]["revision"] = json!(2),
            "duplicate" => {
                let pin = value["connections"][0].clone();
                value["connections"].as_array_mut().unwrap().push(pin);
            }
            "token" => value["access_token"] = json!("never-a-credential"),
            "nested-token" => {
                value["connections"][0]["refresh_token"] = json!("never-a-credential")
            }
            _ => unreachable!(),
        }
        assert!(fixture.restore(&value.to_string()).is_err(), "{changed}");
    }
    assert!(
        fixture
            .restore(&" ".repeat(MAX_SNAPSHOT_BYTES + 1))
            .is_err()
    );
    assert!(fixture.restore("{").is_err());
}
