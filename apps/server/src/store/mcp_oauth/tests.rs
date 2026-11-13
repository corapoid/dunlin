use super::*;
use dunlin_domain::collaboration::WorkspaceRole;

const BINDING: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
fn setup(path: &std::path::Path) -> (Store, AuthContext) {
    let store = Store::open(path).unwrap();
    let owner = store.create_user("owner", "admin").unwrap();
    let team = store.create_workspace(&owner, "OAuth test").unwrap();
    let context = AuthContext {
        user_id: owner.id,
        workspace_id: team.id,
        connection_id: uuid::Uuid::now_v7().to_string(),
        connection_revision: 1,
    };
    (store, context)
}
fn installed(store: &Store, context: &AuthContext) -> CredentialRecord {
    let intent = store.begin_install(context, BINDING).unwrap();
    store
        .prepare_write(context, BINDING, &intent, DIGEST)
        .unwrap();
    store
        .claim_write(context, BINDING, &intent, DIGEST)
        .unwrap();
    store
        .acknowledge_write(context, BINDING, &intent, DIGEST)
        .unwrap();
    store.commit(context, BINDING, &intent, DIGEST).unwrap();
    store.active(context, BINDING).unwrap()
}
#[test]
fn mcp_oauth_journal_crash_before_or_after_vault_write_never_reactivates_refresh() {
    for staged_written in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("relay.db");
        let (store, context) = setup(&path);
        let record = installed(&store, &context);
        let intent = store.begin_refresh(&context, BINDING, &record).unwrap();
        if staged_written {
            // Crash after the provider rotated and the private store was written,
            // but before SQL commit. SQL must not expose either generation.
            std::fs::write(
                directory.path().join("fake-private-vault"),
                &intent.staged_reference,
            )
            .unwrap();
        }
        drop(store);
        let restarted = Store::open(&path).unwrap();
        assert!(restarted.active(&context, BINDING).is_err());
        assert!(restarted.begin_refresh(&context, BINDING, &record).is_err());
        assert!(restarted.begin_install(&context, BINDING).is_err());
        assert_eq!(
            restarted
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM mcp_oauth_secret_references",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
    }
}
#[test]
fn mcp_oauth_journal_competing_connections_and_revision_fence_late_writers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let (store, mut context) = setup(&path);
    let record = installed(&store, &context);
    let competing = Store::open(&path).unwrap();
    let intent = store.begin_refresh(&context, BINDING, &record).unwrap();
    assert!(competing.begin_refresh(&context, BINDING, &record).is_err());
    let stale = context.clone();
    context.connection_revision = competing.disable(&context).unwrap();
    assert_eq!(context.connection_revision, 2);
    assert!(store.commit(&stale, BINDING, &intent, DIGEST).is_err());
    assert!(store.commit(&context, BINDING, &intent, DIGEST).is_err());
    assert!(store.active(&stale, BINDING).is_err());
    let next = competing.begin_install(&context, BINDING).unwrap();
    assert_ne!(next.staged_reference, intent.staged_reference);
    assert_eq!(next.generation, 2);
    competing
        .prepare_write(&context, BINDING, &next, DIGEST)
        .unwrap();
    competing.commit(&context, BINDING, &next, DIGEST).unwrap();
    assert!(store.commit(&stale, BINDING, &intent, DIGEST).is_err());
    assert_eq!(
        store.active(&context, BINDING).unwrap().secret_reference,
        next.staged_reference
    );
    drop(store);
    drop(competing);
    let restarted = Store::open(&path).unwrap();
    assert_eq!(restarted.active(&context, BINDING).unwrap().generation, 2);
    // Retain the late-writer tombstone as well as the old and new references.
    assert_eq!(
        restarted
            .connection
            .query_row(
                "SELECT COUNT(*) FROM mcp_oauth_secret_references",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        3
    );
}
#[test]
fn mcp_oauth_journal_checks_current_owner_membership_binding_and_all_intent_fields() {
    let directory = tempfile::tempdir().unwrap();
    let (store, context) = setup(&directory.path().join("relay.db"));
    let record = installed(&store, &context);
    let other = store.create_user("other", "admin").unwrap();
    store
        .set_team_member(
            &context.user_id,
            &context.workspace_id,
            &other.id,
            WorkspaceRole::Admin,
            true,
            None,
        )
        .unwrap();
    let mut other_context = context.clone();
    other_context.user_id = other.id;
    assert!(store.active(&other_context, BINDING).is_err());
    assert!(store.disable(&other_context).is_err());
    assert!(store.begin_install(&other_context, BINDING).is_err());
    assert!(store.begin_refresh(&context, DIGEST, &record).is_err());
    let intent = store.begin_refresh(&context, BINDING, &record).unwrap();
    for field in 0..3 {
        let mut forged = intent.clone();
        match field {
            0 => forged.generation += 1,
            1 => forged.operation_id.push('x'),
            _ => forged.staged_reference.push('x'),
        }
        assert!(store.commit(&context, BINDING, &forged, DIGEST).is_err());
    }
    // A role change/deactivation during the external OAuth request prevents commit.
    store.connection.execute("UPDATE team_memberships SET active=0,revision=revision+1 WHERE workspace_id=?1 AND user_id=?2",
        params![context.workspace_id,context.user_id]).unwrap();
    assert!(store.commit(&context, BINDING, &intent, DIGEST).is_err());
    assert!(store.active(&context, BINDING).is_err());
    assert!(store.disable(&context).is_err());
}
#[test]
fn mcp_oauth_journal_commit_is_single_use_and_generation_is_cas_bound() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let (store, context) = setup(&path);
    let record = installed(&store, &context);
    let intent = store.begin_refresh(&context, BINDING, &record).unwrap();
    store
        .prepare_write(&context, BINDING, &intent, DIGEST)
        .unwrap();
    store.commit(&context, BINDING, &intent, DIGEST).unwrap();
    assert!(store.commit(&context, BINDING, &intent, DIGEST).is_err());
    assert!(store.begin_refresh(&context, BINDING, &record).is_err());
    drop(store);
    let restarted = Store::open(&path).unwrap();
    let active = restarted.active(&context, BINDING).unwrap();
    assert_eq!(active.generation, 2);
    assert_eq!(active.secret_reference, intent.staged_reference);
    assert_ne!(active.secret_reference, record.secret_reference);
}
#[test]
fn mcp_oauth_journal_migration_preserves_membership_and_rejects_schema_collision_atomically() {
    for collision in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("relay.db");
        let (store, context) = setup(&path);
        store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; PRAGMA user_version=37;").unwrap();
        if collision {
            store.connection.execute_batch("CREATE TABLE mcp_oauth_secret_references(marker TEXT); INSERT INTO mcp_oauth_secret_references VALUES('preserve');").unwrap();
        }
        drop(store);
        let opened = Store::open(&path);
        if collision {
            assert!(opened.is_err());
            let raw = Connection::open(&path).unwrap();
            assert_eq!(
                raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                37
            );
            assert_eq!(
                raw.query_row("SELECT marker FROM mcp_oauth_secret_references", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
                "preserve"
            );
            assert_eq!(
                raw.query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE name='mcp_oauth_connections'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        } else {
            let migrated = opened.unwrap();
            assert!(
                migrated
                    .team_membership(&context.workspace_id, &context.user_id)
                    .unwrap()
                    .unwrap()
                    .active
            );
            assert_eq!(
                migrated
                    .connection
                    .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                crate::store::SCHEMA_VERSION
            );
            installed(&migrated, &context);
        }
    }
}

// Uses the production SecretStore interface, with an isolated in-memory value
// backend. Every read/write proves that no Store lock spans secret operations.
struct CheckedOAuthSecrets {
    values: dunlin_vault::store::MemoryStore,
    journal: std::sync::Mutex<std::sync::Weak<std::sync::Mutex<Store>>>,
    context: AuthContext,
    fault: &'static str,
    writes: std::sync::atomic::AtomicUsize,
}
impl CheckedOAuthSecrets {
    fn unlocked(&self) -> std::sync::Arc<std::sync::Mutex<Store>> {
        let journal = self.journal.lock().unwrap().upgrade().unwrap();
        assert!(
            journal.try_lock().is_ok(),
            "Store lock held across secret storage"
        );
        journal
    }
}
impl dunlin_vault::store::SecretStore for CheckedOAuthSecrets {
    fn get(&self, id: &str) -> Result<zeroize::Zeroizing<String>, String> {
        let _journal = self.unlocked();
        self.values.get(id)
    }
    fn set(&self, id: &str, value: &str) -> Result<(), String> {
        let journal = self.unlocked();
        let refresh = self
            .writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            == 1;
        if refresh && self.fault == "vault" {
            return Err("fixture-private-keyring-error".into());
        }
        let expected: String = journal
            .lock()
            .unwrap()
            .connection
            .query_row(
                "SELECT sha256 FROM mcp_oauth_secret_references WHERE reference=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        use sha2::Digest;
        assert_eq!(
            expected,
            hex::encode(sha2::Sha256::digest(value.as_bytes()))
        );
        self.values.set(id, value)?;
        if refresh && self.fault == "vault_after_write" {
            return Err("fixture-private-keyring-error-after-write".into());
        }
        if refresh && self.fault == "fence" {
            journal.disable(&self.context).unwrap();
        }
        Ok(())
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        panic!("no premature cleanup")
    }
    fn get_policy(&self, _: &str) -> Result<zeroize::Zeroizing<String>, String> {
        panic!("OAuth must not access vault tool policies")
    }
    fn set_policy(&self, _: &str, _: &str) -> Result<(), String> {
        panic!("OAuth must not register vault tools")
    }
    fn delete_policy(&self, _: &str) -> Result<(), String> {
        panic!("OAuth must not change vault tools")
    }
}

#[test]
fn mcp_oauth_journal_real_http_refresh_and_restart_cover_vault_failure_and_late_writer() {
    use dunlin_mcp_client::oauth::{Discovery, Registration, ResourceMetadata};
    use dunlin_vault::oauth::OAuthCredentials;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };
    for fault in [
        "none",
        "response",
        "vault",
        "vault_after_write",
        "fence",
        "http_disconnect",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("relay.db");
        let (store, context) = setup(&path);
        let journal = Arc::new(Mutex::new(store));
        let secrets = Arc::new(CheckedOAuthSecrets {
            values: dunlin_vault::store::MemoryStore::new(),
            journal: Mutex::new(Arc::downgrade(&journal)),
            context: context.clone(),
            fault,
            writes: std::sync::atomic::AtomicUsize::new(0),
        });
        let credentials = OAuthCredentials::new(journal.clone(), secrets.clone());
        let request_journal = journal.clone();
        let request_context = context.clone();
        let database = path.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            for index in 0..2 {
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
                        _ => panic!("expected OAuth HTTP request"),
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
                assert!(headers.starts_with("POST /token HTTP/1.1\r\n"));
                let size: usize = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                assert!(size < 32768);
                let mut body = vec![0; size];
                reader.read_exact(&mut body).unwrap();
                if index == 1 {
                    assert!(
                        request_journal.try_lock().is_ok(),
                        "Store lock held across provider HTTP"
                    );
                    // A separate SQLite connection observes a committed intent
                    // before the provider can rotate the refresh token.
                    let observer = Connection::open(&database).unwrap();
                    assert_eq!(
                        observer
                            .query_row("SELECT state FROM mcp_oauth_connections", [], |r| r
                                .get::<_, String>(0))
                            .unwrap(),
                        "refreshing"
                    );
                    assert!(
                        String::from_utf8(body)
                            .unwrap()
                            .contains("refresh_token=fixture-refresh-one")
                    );
                    if fault == "http_disconnect" {
                        // Another request can disconnect while provider HTTP is
                        // in flight; its revision fence defeats the late CAS.
                        request_journal.disable(&request_context).unwrap();
                    }
                    if fault == "response" {
                        continue;
                    }
                }
                let value = if index == 0 { "one" } else { "two" };
                let body=serde_json::json!({"access_token":format!("fixture-access-{value}"),"refresh_token":format!("fixture-refresh-{value}"),"token_type":"Bearer","expires_in":3600}).to_string();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let endpoint = format!("{origin}/mcp");
        let issuer = format!("{origin}/issuer");
        let discovery = Discovery::new(&endpoint, &[]).unwrap();
        let resource = ResourceMetadata {
            resource: endpoint,
            authorization_servers: vec![issuer.clone()],
            scopes_supported: None,
        };
        let metadata=serde_json::from_value(serde_json::json!({"issuer":issuer,"authorization_endpoint":format!("{origin}/authorize"),"token_endpoint":format!("{origin}/token"),"response_types_supported":["code"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none"]})).unwrap();
        let flow = discovery
            .authorization_flow(
                &resource,
                metadata,
                Registration::public(&issuer, "client", "http://127.0.0.1/callback").unwrap(),
            )
            .unwrap();
        let request = flow.begin(context.clone(), vec![]).unwrap();
        let state = request
            .url
            .split('?')
            .nth(1)
            .unwrap()
            .split('&')
            .find_map(|v| v.strip_prefix("state="))
            .unwrap();
        let token = flow
            .finish(
                &context,
                &format!("http://127.0.0.1/callback?state={state}&code=fixture-code"),
            )
            .unwrap();
        let original = credentials.install(&flow, &context, token).unwrap();
        let result = credentials.refresh(&flow, &context);
        assert_eq!(result.is_ok(), fault == "none");
        if let Err(error) = &result {
            assert!(!error.to_string().contains("fixture-private-keyring-error"));
        }
        worker.join().unwrap();
        drop(credentials);
        drop(Arc::try_unwrap(journal).ok().unwrap().into_inner().unwrap());
        let restarted = Arc::new(Mutex::new(Store::open(&path).unwrap()));
        *secrets.journal.lock().unwrap() = Arc::downgrade(&restarted);
        let restored = OAuthCredentials::new(restarted.clone(), secrets.clone());
        let (revision, binding, generation, hash): (i64, String, i64, Option<String>) = restarted.lock().unwrap().connection.query_row(
            "SELECT credential_revision,binding,generation,sha256 FROM mcp_oauth_secret_references WHERE generation=2", [],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(
            (revision, binding, generation),
            (1, flow.credential_binding(), 2)
        );
        assert_eq!(
            hash.is_some(),
            !matches!(fault, "response" | "http_disconnect")
        );

        if fault == "none" {
            let bearer = restored.access_token(&flow, &context).unwrap();
            assert_eq!(&*bearer, "fixture-access-two");
            let current = restarted
                .active(&context, &flow.credential_binding())
                .unwrap();
            assert_eq!(current.generation, 2);
            assert_ne!(current.secret_reference, original.secret_reference);
            assert_eq!(restored.disable(&context).unwrap(), 2);
            assert!(restored.access_token(&flow, &context).is_err());
        } else {
            let writes = secrets.writes.load(std::sync::atomic::Ordering::SeqCst);
            assert!(restored.refresh(&flow, &context).is_err());
            assert!(restored.access_token(&flow, &context).is_err());
            assert_eq!(
                secrets.writes.load(std::sync::atomic::Ordering::SeqCst),
                writes
            );
        }
        // Provider secrets must not enter either the database or its WAL.
        for filename in ["relay.db", "relay.db-wal"] {
            if let Ok(bytes) = std::fs::read(directory.path().join(filename)) {
                for token in [
                    b"fixture-access-one".as_slice(),
                    b"fixture-refresh-one",
                    b"fixture-access-two",
                    b"fixture-refresh-two",
                ] {
                    assert!(!bytes.windows(token.len()).any(|window| window == token));
                }
            }
        }
    }
}

#[test]
fn mcp_oauth_journal_reference_limit_rolls_back_without_blocking_active_generation() {
    let directory = tempfile::tempdir().unwrap();
    let (store, context) = setup(&directory.path().join("relay.db"));
    let record = installed(&store, &context);
    store
        .connection
        .execute(
            "WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<255)
        INSERT INTO mcp_oauth_secret_references(reference,connection_id,created_at)
        SELECT 'fixture-retired:'||n,?1,0 FROM numbers",
            [&context.connection_id],
        )
        .unwrap();
    assert!(matches!(
        store.begin_refresh(&context, BINDING, &record),
        Err(PersistenceError::Limit)
    ));
    assert_eq!(store.active(&context, BINDING).unwrap(), record);
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM mcp_oauth_secret_references",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        256
    );
}

#[test]
fn mcp_oauth_shared_journal_poison_never_recovers_credential_access_or_writes() {
    use std::sync::Mutex;
    let directory = tempfile::tempdir().unwrap();
    let (store, context) = setup(&directory.path().join("relay.db"));
    let record = installed(&store, &context);
    let journal = Mutex::new(store);
    let intent = journal.begin_refresh(&context, BINDING, &record).unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = journal.lock().unwrap();
            panic!("injected journal owner failure");
        }))
        .is_err()
    );
    assert!(matches!(
        journal.active(&context, BINDING),
        Err(PersistenceError::Storage)
    ));
    assert!(matches!(
        journal.begin_install(&context, BINDING),
        Err(PersistenceError::Storage)
    ));
    assert!(matches!(
        journal.begin_refresh(&context, BINDING, &record),
        Err(PersistenceError::Storage)
    ));
    assert!(matches!(
        journal.commit(&context, BINDING, &intent, DIGEST),
        Err(PersistenceError::Storage)
    ));
    assert!(matches!(
        journal.disable(&context),
        Err(PersistenceError::Storage)
    ));
}

#[test]
fn mcp_oauth_disconnect_audit_failure_rolls_back_revision_and_leaves_no_success_notification() {
    let directory = tempfile::tempdir().unwrap();
    let (store, context) = setup(&directory.path().join("relay.db"));
    let record = installed(&store, &context);
    let mut notifications = store.subscribe_catalog();
    store.connection.execute_batch("CREATE TRIGGER fail_oauth_audit BEFORE INSERT ON team_audit WHEN NEW.action='mcp_oauth_disconnected' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(store.disable(&context).is_err());
    assert_eq!(store.active(&context, BINDING).unwrap(), record);
    assert!(notifications.try_recv().is_err());
    store
        .connection
        .execute_batch("DROP TRIGGER fail_oauth_audit;")
        .unwrap();
    assert_eq!(store.disable(&context).unwrap(), 2);
    assert!(notifications.try_recv().is_ok());
    assert!(store.active(&context, BINDING).is_err());
}

#[test]
fn mcp_oauth_authorization_prepare_takeover_disconnect_and_stale_install_are_fenced() {
    let directory = tempfile::tempdir().unwrap();
    let (store, mut context) = setup(&directory.path().join("relay.db"));
    assert_eq!(
        store
            .mcp_oauth_prepare(&context, BINDING, "fixture", None, &|_, _| Ok(()))
            .unwrap(),
        1
    );
    store.mcp_oauth_authorizing(&context, BINDING).unwrap();
    assert!(store.mcp_oauth_authorizing(&context, DIGEST).is_err());
    assert!(store.begin_install(&context, DIGEST).is_err());
    let stale = context.clone();
    context.connection_revision = store
        .mcp_oauth_prepare(&context, BINDING, "fixture", None, &|_, _| Ok(()))
        .unwrap();
    assert_eq!(context.connection_revision, 2);
    assert!(store.mcp_oauth_authorizing(&stale, BINDING).is_err());
    assert!(store.begin_install(&stale, BINDING).is_err());
    assert_eq!(store.disable(&context).unwrap(), 3);
    assert!(store.mcp_oauth_authorizing(&context, BINDING).is_err());
    context.connection_revision = 3;
    store
        .mcp_oauth_prepare(&context, BINDING, "fixture", None, &|_, _| Ok(()))
        .unwrap();
    let intent = store.begin_install(&context, BINDING).unwrap();
    store
        .prepare_write(&context, BINDING, &intent, DIGEST)
        .unwrap();
    store.commit(&context, BINDING, &intent, DIGEST).unwrap();
    assert!(
        store
            .mcp_oauth_prepare(&context, BINDING, "fixture", None, &|_, _| Ok(()))
            .is_err()
    );
    assert_eq!(store.active(&context, BINDING).unwrap().generation, 1);
}

#[test]
fn mcp_oauth_migration39_preserves_nonempty_references_and_uncertain_refresh_atomically() {
    for pending in [false, true] {
        for collision in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("relay.db");
            let (store, context) = setup(&path);
            let record = installed(&store, &context);
            if pending {
                store.begin_refresh(&context, BINDING, &record).unwrap();
            }
            // Reconstruct the actual v38 schema, preserving all old columns and refs.
            store.connection.execute_batch("CREATE TEMP TABLE saved_connections AS SELECT connection_id,workspace_id,user_id,revision,binding,generation,state,secret_reference,sha256,operation_id,staged_reference FROM mcp_oauth_connections;
                CREATE TEMP TABLE saved_refs AS SELECT reference,connection_id,created_at FROM mcp_oauth_secret_references;
                DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections;").unwrap();
            store.connection.execute_batch(SCHEMA).unwrap();
            store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; INSERT INTO mcp_oauth_connections SELECT * FROM saved_connections; INSERT INTO mcp_oauth_secret_references SELECT * FROM saved_refs; DROP TABLE saved_connections; DROP TABLE saved_refs; PRAGMA user_version=38;").unwrap();
            if collision {
                store.connection.execute_batch("CREATE TABLE mcp_oauth_secret_references_next(marker TEXT);INSERT INTO mcp_oauth_secret_references_next VALUES('preserve');").unwrap();
            }
            drop(store);
            if collision {
                assert!(Store::open(&path).is_err());
                let raw = Connection::open(&path).unwrap();
                assert_eq!(
                    raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                        .unwrap(),
                    38
                );
                assert_eq!(
                    raw.query_row(
                        "SELECT marker FROM mcp_oauth_secret_references_next",
                        [],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                    "preserve"
                );
                assert_eq!(raw.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE name='mcp_oauth_connections_next'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
                raw.execute_batch("DROP TABLE mcp_oauth_secret_references_next;")
                    .unwrap();
            }
            let migrated = Store::open(&path).unwrap();
            assert_eq!(
                migrated
                    .connection
                    .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                crate::store::SCHEMA_VERSION
            );
            assert_eq!(
                migrated
                    .connection
                    .query_row(
                        "SELECT COUNT(*) FROM mcp_oauth_secret_references",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                if pending { 2 } else { 1 }
            );
            assert_eq!(migrated.active(&context, BINDING).is_err(), pending);
            if !pending {
                assert_eq!(migrated.active(&context, BINDING).unwrap(), record);
            }
            assert!(
                migrated
                    .connection
                    .prepare("PRAGMA foreign_key_check")
                    .unwrap()
                    .query([])
                    .unwrap()
                    .next()
                    .unwrap()
                    .is_none()
            );
            let status = migrated
                .mcp_oauth_connection(
                    &context.user_id,
                    &context.workspace_id,
                    &context.connection_id,
                )
                .unwrap()
                .unwrap();
            assert!(status.provider_id.is_none());
        }
    }
}

#[test]
fn mcp_oauth_checked_journal_checks_external_authority_inside_the_same_sqlite_cut() {
    use std::cell::Cell;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let (store, context) = setup(&path);
    let first = installed(&store, &context);
    let other = Connection::open(&path).unwrap();
    other.busy_timeout(std::time::Duration::ZERO).unwrap();
    let allowed = Cell::new(true);
    let checked = Cell::new(0);
    let check = |store: &Store, _: &AuthContext| {
        assert!(!store.connection.is_autocommit());
        let result = other.execute(
            "UPDATE team_memberships SET active=0 WHERE workspace_id=?1 AND user_id=?2",
            params![context.workspace_id, context.user_id],
        );
        assert!(
            matches!(result,Err(rusqlite::Error::SqliteFailure(error,_)) if error.code==rusqlite::ErrorCode::DatabaseBusy)
        );
        checked.set(checked.get() + 1);
        if allowed.get() {
            Ok(())
        } else {
            Err(PersistenceError::Unauthorized)
        }
    };
    let journal = store.mcp_oauth_journal(&check);
    assert_eq!(journal.active(&context, BINDING).unwrap(), first);
    let intent = journal.begin_refresh(&context, BINDING, &first).unwrap();
    allowed.set(false);
    assert!(journal.commit(&context, BINDING, &intent, DIGEST).is_err());
    allowed.set(true);
    journal
        .prepare_write(&context, BINDING, &intent, DIGEST)
        .unwrap();
    journal.commit(&context, BINDING, &intent, DIGEST).unwrap();
    assert_eq!(journal.active(&context, BINDING).unwrap().generation, 2);
    assert_eq!(journal.disable(&context).unwrap(), 2);
    assert_eq!(checked.get(), 7);
}

#[test]
fn mcp_oauth_display_name_is_private_bounded_and_preserved_during_reauthorization() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let (store, mut context) = setup(&path);
    assert!(
        store
            .mcp_oauth_prepare(&context, BINDING, "fixture", Some("\nprivate"), &|_, _| Ok(
                ()
            ))
            .is_err()
    );
    assert_eq!(
        store
            .mcp_oauth_prepare(
                &context,
                BINDING,
                "fixture",
                Some("Konto służbowe"),
                &|_, _| Ok(())
            )
            .unwrap(),
        1
    );
    let record = store
        .mcp_oauth_connection(
            &context.user_id,
            &context.workspace_id,
            &context.connection_id,
        )
        .unwrap()
        .unwrap();
    assert_eq!(record.display_name.as_deref(), Some("Konto służbowe"));
    assert!(
        store
            .mcp_oauth_prepare(
                &context,
                BINDING,
                "fixture",
                Some("Inne konto"),
                &|_, _| Ok(())
            )
            .is_err()
    );
    context.connection_revision = store.disable(&context).unwrap();
    store
        .mcp_oauth_prepare(&context, BINDING, "fixture", None, &|_, _| Ok(()))
        .unwrap();
    let record = store
        .mcp_oauth_connection(
            &context.user_id,
            &context.workspace_id,
            &context.connection_id,
        )
        .unwrap()
        .unwrap();
    assert_eq!(record.display_name.as_deref(), Some("Konto służbowe"));
}
#[test]
fn mcp_oauth_migration40_keeps_tokens_references_and_rolls_back_duplicate_column_collision() {
    for collision in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("relay.db");
        let (store, context) = setup(&path);
        let record = installed(&store, &context);
        store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP INDEX IF EXISTS mcp_oauth_retention; ALTER TABLE mcp_oauth_secret_references DROP COLUMN delete_verified; ALTER TABLE mcp_oauth_secret_references DROP COLUMN retired; ALTER TABLE mcp_oauth_secret_references DROP COLUMN as_confirmed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN cleanup_attempted_revision; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_acknowledged; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_claimed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_operation_id; ALTER TABLE mcp_oauth_secret_references DROP COLUMN writer_protected; ALTER TABLE mcp_oauth_connections DROP COLUMN cleanup; ALTER TABLE mcp_oauth_connections DROP COLUMN revocation; ALTER TABLE mcp_oauth_secret_references DROP COLUMN credential_revision; ALTER TABLE mcp_oauth_secret_references DROP COLUMN binding; ALTER TABLE mcp_oauth_secret_references DROP COLUMN generation; ALTER TABLE mcp_oauth_secret_references DROP COLUMN sha256; ALTER TABLE mcp_oauth_connections DROP COLUMN display_name;DROP TABLE IF EXISTS interactive_mcp_inputs; PRAGMA user_version=39;").unwrap();
        if collision {
            store.connection.execute_batch("ALTER TABLE mcp_oauth_connections ADD COLUMN display_name INTEGER;UPDATE mcp_oauth_connections SET display_name=777;").unwrap();
        }
        drop(store);
        if collision {
            assert!(Store::open(&path).is_err());
            let raw = Connection::open(&path).unwrap();
            assert_eq!(
                raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                39
            );
            assert_eq!(
                raw.query_row("SELECT display_name FROM mcp_oauth_connections", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap(),
                777
            );
            raw.execute_batch("ALTER TABLE mcp_oauth_connections DROP COLUMN display_name;")
                .unwrap();
        }
        let migrated = Store::open(&path).unwrap();
        assert_eq!(migrated.active(&context, BINDING).unwrap(), record);
        assert_eq!(
            migrated
                .connection
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            crate::store::SCHEMA_VERSION
        );
        assert!(
            migrated
                .mcp_oauth_connection(
                    &context.user_id,
                    &context.workspace_id,
                    &context.connection_id
                )
                .unwrap()
                .unwrap()
                .display_name
                .is_none()
        );
    }
}

#[test]
fn mcp_oauth_prepared_envelope_is_immutable_and_survives_revision_fence_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let (store, context) = setup(&path);
    let intent = store.begin_install(&context, BINDING).unwrap();
    assert!(store.commit(&context, BINDING, &intent, DIGEST).is_err());
    store
        .prepare_write(&context, BINDING, &intent, DIGEST)
        .unwrap();
    store
        .prepare_write(&context, BINDING, &intent, DIGEST)
        .unwrap();
    assert!(
        store
            .prepare_write(&context, BINDING, &intent, BINDING)
            .is_err()
    );
    assert!(store.commit(&context, BINDING, &intent, BINDING).is_err());
    for field in 0..3 {
        let mut forged = intent.clone();
        match field {
            0 => forged.generation += 1,
            1 => forged.operation_id.push('x'),
            _ => forged.staged_reference.push('x'),
        }
        assert!(
            store
                .prepare_write(&context, BINDING, &forged, DIGEST)
                .is_err()
        );
    }
    let revision = store.disable(&context).unwrap();
    let mut next = context.clone();
    next.connection_revision = revision;
    assert!(
        store
            .prepare_write(&context, BINDING, &intent, DIGEST)
            .is_err()
    );
    assert!(
        store
            .prepare_write(&next, BINDING, &intent, DIGEST)
            .is_err()
    );
    drop(store);
    let restarted = Store::open(&path).unwrap();
    let metadata: (i64, String, i64, String) = restarted.connection.query_row(
        "SELECT credential_revision,binding,generation,sha256 FROM mcp_oauth_secret_references WHERE reference=?1",
        [&intent.staged_reference], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(metadata, (1, BINDING.into(), 1, DIGEST.into()));
    assert!(
        restarted
            .commit(&context, BINDING, &intent, DIGEST)
            .is_err()
    );
    assert!(restarted.active(&next, BINDING).is_err());
}

#[test]
fn mcp_oauth_metadata_migration_backfills_only_proven_envelopes_and_is_atomic() {
    for state in ["active", "refreshing", "disabled"] {
        for collision in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("relay.db");
            let (store, context) = setup(&path);
            let record = installed(&store, &context);
            let staged = if state == "refreshing" {
                Some(store.begin_refresh(&context, BINDING, &record).unwrap())
            } else {
                None
            };
            if state == "disabled" {
                store.disable(&context).unwrap();
            }
            store
                .connection
                .execute_batch(
                    "DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP INDEX IF EXISTS mcp_oauth_retention; ALTER TABLE mcp_oauth_secret_references DROP COLUMN delete_verified; ALTER TABLE mcp_oauth_secret_references DROP COLUMN retired; ALTER TABLE mcp_oauth_secret_references DROP COLUMN as_confirmed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN cleanup_attempted_revision; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_acknowledged; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_claimed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_operation_id; ALTER TABLE mcp_oauth_secret_references DROP COLUMN writer_protected; ALTER TABLE mcp_oauth_connections DROP COLUMN cleanup; ALTER TABLE mcp_oauth_connections DROP COLUMN revocation; ALTER TABLE mcp_oauth_secret_references DROP COLUMN credential_revision;
                ALTER TABLE mcp_oauth_secret_references DROP COLUMN binding;
                ALTER TABLE mcp_oauth_secret_references DROP COLUMN generation;
                ALTER TABLE mcp_oauth_secret_references DROP COLUMN sha256;
                PRAGMA user_version=41;",
                )
                .unwrap();
            if collision {
                // The first ALTER must roll back if a later column collides.
                store
                    .connection
                    .execute_batch(
                        "ALTER TABLE mcp_oauth_secret_references ADD COLUMN binding TEXT;
                    UPDATE mcp_oauth_secret_references SET binding='preserve';",
                    )
                    .unwrap();
            }
            drop(store);
            if collision {
                assert!(Store::open(&path).is_err());
                let raw = Connection::open(&path).unwrap();
                assert_eq!(
                    raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                        .unwrap(),
                    41
                );
                assert_eq!(
                    raw.query_row(
                        "SELECT binding FROM mcp_oauth_secret_references LIMIT 1",
                        [],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                    "preserve"
                );
                assert!(
                    raw.prepare("SELECT credential_revision FROM mcp_oauth_secret_references")
                        .is_err()
                );
                raw.execute_batch("ALTER TABLE mcp_oauth_secret_references DROP COLUMN binding;")
                    .unwrap();
            }
            let migrated = Store::open(&path).unwrap();
            let (revision, hash): (Option<i64>, Option<String>) = migrated.connection.query_row(
                "SELECT credential_revision,sha256 FROM mcp_oauth_secret_references WHERE reference=?1",
                [&record.secret_reference], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
            if state == "disabled" {
                assert_eq!((revision, hash), (None, None));
            } else {
                assert_eq!((revision, hash), (Some(1), Some(DIGEST.into())));
            }
            if let Some(staged) = staged {
                let metadata: (i64, String, i64, Option<String>) = migrated.connection.query_row(
                    "SELECT credential_revision,binding,generation,sha256 FROM mcp_oauth_secret_references WHERE reference=?1",
                    [&staged.staged_reference], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
                assert_eq!(metadata, (1, BINDING.into(), 2, None));
            }
        }
    }
}

#[test]
fn mcp_oauth_revocation_crash_and_one_use_claim_never_restore_positive_authority() {
    use dunlin_mcp_client::oauth::persistence::RevocationJournal;
    for claimed in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("revoke.db");
        let (store, context) = setup(&path);
        let original = installed(&store, &context);
        let intent = store.begin_revocation(&context, BINDING).unwrap();
        assert_eq!(intent.connection_revision, 2);
        assert!(store.active(&context, BINDING).is_err());
        assert!(store.check_revocation_target(&context, &intent, 0).is_err());
        if claimed {
            store.claim_revocation_target(&context, &intent, 0).unwrap();
            assert!(store.claim_revocation_target(&context, &intent, 0).is_err());
        }
        drop(store);
        let restarted = Store::open(&path).unwrap();
        let mut next = context.clone();
        next.connection_revision = 2;
        assert!(restarted.begin_install(&next, BINDING).is_err());
        assert!(restarted.mcp_oauth_can_authorize(&next).is_err());
        assert!(restarted.begin_revocation(&context, BINDING).is_err());
        assert!(restarted.begin_revocation(&next, BINDING).is_err());
        assert!(
            restarted
                .begin_refresh(&context, BINDING, &original)
                .is_err()
        );
        let value = serde_json::to_value(
            restarted
                .mcp_oauth_connection(
                    &context.user_id,
                    &context.workspace_id,
                    &context.connection_id,
                )
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(value["revocation"]["state"], "unfinished");
        assert_eq!(value["revocation"]["confirmed"], 0);
        assert!(!value.to_string().contains(&original.secret_reference));
    }
}

#[test]
fn mcp_oauth_revocation_target_authority_and_receipts_are_bound_to_the_whole_snapshot() {
    use dunlin_mcp_client::oauth::persistence::RevocationJournal;
    let dir = tempfile::tempdir().unwrap();
    let (store, context) = setup(&dir.path().join("db"));
    let original = installed(&store, &context);
    let refresh = store.begin_refresh(&context, BINDING, &original).unwrap();
    let mut intent = store.begin_revocation(&context, BINDING).unwrap();
    assert_eq!(intent.targets.len(), 2);
    assert_eq!(intent.targets.iter().filter(|v| v.is_some()).count(), 1);
    let old = intent.operation_id.clone();
    intent.operation_id.push('x');
    assert!(store.claim_revocation_target(&context, &intent, 0).is_err());
    intent.operation_id = old;
    for n in 0..2 {
        store.claim_revocation_target(&context, &intent, n).unwrap();
        if intent.targets[n].is_some() {
            store
                .confirm_revocation_target(&context, &intent, n)
                .unwrap();
        } else {
            assert!(
                store
                    .confirm_revocation_target(&context, &intent, n)
                    .is_err()
            );
        }
    }
    let summary = store.finish_revocation(&context, &intent).unwrap();
    assert_eq!((summary.confirmed, summary.unconfirmed), (1, 1));
    assert!(!summary.confirmed_known_tokens);
    assert!(store.finish_revocation(&context, &intent).is_err());
    assert!(
        store
            .prepare_write(&context, BINDING, &refresh, DIGEST)
            .is_err()
    );
    assert!(store.commit(&context, BINDING, &refresh, DIGEST).is_err());
    let mut next = context.clone();
    next.connection_revision = 2;
    assert!(store.mcp_oauth_can_authorize(&next).is_ok());
}

#[test]
fn mcp_oauth_revocation_migration43_preserves_credentials_and_rolls_back_collision() {
    for collision in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("upgrade.db");
        let (store, context) = setup(&path);
        let original = installed(&store, &context);
        store
            .connection
            .execute_batch(
                "DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP INDEX IF EXISTS mcp_oauth_retention; ALTER TABLE mcp_oauth_secret_references DROP COLUMN delete_verified; ALTER TABLE mcp_oauth_secret_references DROP COLUMN retired; ALTER TABLE mcp_oauth_secret_references DROP COLUMN as_confirmed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN cleanup_attempted_revision; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_acknowledged; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_claimed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_operation_id; ALTER TABLE mcp_oauth_secret_references DROP COLUMN writer_protected; ALTER TABLE mcp_oauth_connections DROP COLUMN cleanup; ALTER TABLE mcp_oauth_connections DROP COLUMN revocation; PRAGMA user_version=42;",
            )
            .unwrap();
        if collision {
            store.connection.execute_batch("ALTER TABLE mcp_oauth_connections ADD COLUMN revocation INTEGER; UPDATE mcp_oauth_connections SET revocation=777;").unwrap();
        }
        drop(store);
        if collision {
            assert!(Store::open(&path).is_err());
            let raw = Connection::open(&path).unwrap();
            assert_eq!(
                raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                42
            );
            assert_eq!(
                raw.query_row("SELECT revocation FROM mcp_oauth_connections", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                777
            );
            raw.execute_batch("ALTER TABLE mcp_oauth_connections DROP COLUMN revocation;")
                .unwrap();
        }
        let migrated = Store::open(&path).unwrap();
        assert_eq!(migrated.active(&context, BINDING).unwrap(), original);
        let row = serde_json::to_value(
            migrated
                .mcp_oauth_connection(
                    &context.user_id,
                    &context.workspace_id,
                    &context.connection_id,
                )
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(row.get("revocation").is_none());
    }
}

#[test]
fn mcp_oauth_revocation_without_any_proven_issued_envelope_is_not_confirmation() {
    use dunlin_mcp_client::oauth::persistence::RevocationJournal;
    let directory = tempfile::tempdir().unwrap();
    let (store, context) = setup(&directory.path().join("db"));
    store
        .mcp_oauth_prepare(&context, BINDING, "fixture", None, &|_, _| Ok(()))
        .unwrap();
    let intent = store.begin_revocation(&context, BINDING).unwrap();
    assert!(intent.targets.is_empty());
    let summary = store.finish_revocation(&context, &intent).unwrap();
    assert!(!summary.confirmed_known_tokens);
    assert_eq!((summary.confirmed, summary.unconfirmed), (0, 0));
    let mut next = context.clone();
    next.connection_revision = 2;
    assert!(store.mcp_oauth_can_authorize(&next).is_ok());
}

#[test]
fn mcp_oauth_revocation_retry_retains_confirmed_progress_across_restart_and_registration_change() {
    use dunlin_mcp_client::oauth::persistence::RevocationJournal;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("progress.db");
    let (store, context) = setup(&path);
    let first = installed(&store, &context);
    let refresh = store.begin_refresh(&context, BINDING, &first).unwrap();
    store
        .prepare_write(&context, BINDING, &refresh, DIGEST)
        .unwrap();
    store.commit(&context, BINDING, &refresh, DIGEST).unwrap();
    let intent = store.begin_revocation(&context, BINDING).unwrap();
    store.claim_revocation_target(&context, &intent, 0).unwrap();
    store
        .confirm_revocation_target(&context, &intent, 0)
        .unwrap();
    // The bounded loop can end with untouched targets, not only failed HTTP.
    let partial = store.finish_revocation(&context, &intent).unwrap();
    assert_eq!((partial.confirmed, partial.unconfirmed), (1, 1));
    drop(store);
    let store = Store::open(&path).unwrap();
    let mut current = context.clone();
    current.connection_revision = 2;
    let mut retry = store.begin_revocation(&current, BINDING).unwrap();
    assert_eq!(retry.already_confirmed, vec![false, true]);
    retry.already_confirmed[1] = false;
    assert!(store.claim_revocation_target(&current, &retry, 0).is_err());
    retry.already_confirmed[1] = true;
    assert!(store.claim_revocation_target(&current, &retry, 1).is_err());
    store.claim_revocation_target(&current, &retry, 0).unwrap();
    store
        .confirm_revocation_target(&current, &retry, 0)
        .unwrap();
    let completed = store.finish_revocation(&current, &retry).unwrap();
    assert!(completed.confirmed_known_tokens);
    assert_eq!(completed.confirmed, 2);
    current.connection_revision = 3;
    assert!(store.begin_revocation(&current, BINDING).is_err());
    current.connection_revision = store
        .mcp_oauth_prepare(&current, DIGEST, "fixture", None, &|_, _| Ok(()))
        .unwrap();
    assert_eq!(current.connection_revision, 4);
    let installed = store.begin_install(&current, DIGEST).unwrap();
    store
        .prepare_write(&current, DIGEST, &installed, DIGEST)
        .unwrap();
    store.commit(&current, DIGEST, &installed, DIGEST).unwrap();
    let changed = store.begin_revocation(&current, DIGEST).unwrap();
    assert_eq!(
        changed
            .already_confirmed
            .iter()
            .filter(|confirmed| **confirmed)
            .count(),
        2
    );
    // Old AS acknowledgements remain valid, without sending its credentials to
    // a new registration. Only the newly installed envelope needs provider I/O.
    assert_eq!(
        changed
            .targets
            .iter()
            .filter(|target| target.is_none())
            .count(),
        2
    );
    assert_eq!(
        changed
            .targets
            .iter()
            .filter(|target| target.is_some())
            .count(),
        1
    );
}

#[test]
fn mcp_oauth_revocation_retry_prioritizes_unattempted_records_over_repeated_unknowns() {
    use dunlin_mcp_client::oauth::persistence::RevocationJournal;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fair.db");
    let (store, context) = setup(&path);
    let mut active = installed(&store, &context);
    for _ in 0..2 {
        let intent = store.begin_refresh(&context, BINDING, &active).unwrap();
        store
            .prepare_write(&context, BINDING, &intent, DIGEST)
            .unwrap();
        store.commit(&context, BINDING, &intent, DIGEST).unwrap();
        active = store.active(&context, BINDING).unwrap();
    }
    let first = store.begin_revocation(&context, BINDING).unwrap();
    let references = first
        .targets
        .iter()
        .map(|target| target.as_ref().unwrap().record.secret_reference.clone())
        .collect::<Vec<_>>();
    store.claim_revocation_target(&context, &first, 0).unwrap();
    store.finish_revocation(&context, &first).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let mut current = context.clone();
    current.connection_revision = 2;
    let second = store.begin_revocation(&current, BINDING).unwrap();
    assert_eq!(
        second.targets[0].as_ref().unwrap().record.secret_reference,
        references[1]
    );
    store.claim_revocation_target(&current, &second, 0).unwrap();
    store.finish_revocation(&current, &second).unwrap();
    current.connection_revision = 3;
    let third = store.begin_revocation(&current, BINDING).unwrap();
    assert_eq!(
        third.targets[0].as_ref().unwrap().record.secret_reference,
        references[2]
    );
}

#[test]
fn mcp_oauth_revocation_keeps_audit_bounded_and_still_fences_credentials_at_capacity() {
    use dunlin_mcp_client::oauth::persistence::{CleanupJournal, RevocationJournal};
    let store = Store::open_in_memory().unwrap();
    let owner = store.create_user("audit-revoke", "admin").unwrap();
    let workspace = store.create_workspace(&owner, "audit capacity").unwrap();
    let context = AuthContext {
        user_id: owner.id,
        workspace_id: workspace.id,
        connection_id: uuid::Uuid::now_v7().to_string(),
        connection_revision: 1,
    };
    installed(&store, &context);
    let count: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM team_audit", [], |r| r.get(0))
        .unwrap();
    store
        .connection
        .execute(
            "WITH RECURSIVE fill(n) AS(SELECT 1 UNION ALL SELECT n+1 FROM fill WHERE n<?3)
        INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at)
        SELECT ?1,?2,?2,'capacity fixture',1,0 FROM fill",
            params![context.workspace_id, context.user_id, 100_000 - count],
        )
        .unwrap();
    assert!(store.team_audit_capacity().is_err());
    let lease = store.mcp_oauth_revocation_lease().unwrap();
    let intent = store.begin_revocation(&context, BINDING).unwrap();
    assert_eq!(intent.connection_revision, 2);
    assert!(store.active(&context, BINDING).is_err());
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM team_audit", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        100_000
    );
    let row = serde_json::to_value(
        store
            .mcp_oauth_connection(
                &context.user_id,
                &context.workspace_id,
                &context.connection_id,
            )
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(row["status"], "disabled");
    assert_eq!(row["revocation"]["state"], "unfinished");
    store.claim_revocation_target(&context, &intent, 0).unwrap();
    store
        .confirm_revocation_target(&context, &intent, 0)
        .unwrap();
    drop(lease);
    let lease = store.mcp_oauth_revocation_lease().unwrap();
    let current_context = AuthContext {
        connection_revision: 2,
        ..context.clone()
    };
    assert!(
        store
            .mcp_oauth_reconcile_revocation(&current_context, &lease, &|_, _| Ok(()))
            .unwrap()
            .confirmed_known_tokens
    );
    drop(lease);
    let mut current = context.clone();
    current.connection_revision = 2;
    assert_eq!(
        store.begin_cleanup(&current, 0).unwrap().references.len(),
        1
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM team_audit", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        100_000
    );
}

fn cleanup_ready(path: &std::path::Path) -> (Store, AuthContext, CredentialRecord) {
    use dunlin_mcp_client::oauth::persistence::RevocationJournal;
    let (store, mut context) = setup(path);
    let record = installed(&store, &context);
    let intent = store.begin_revocation(&context, BINDING).unwrap();
    store.claim_revocation_target(&context, &intent, 0).unwrap();
    store
        .confirm_revocation_target(&context, &intent, 0)
        .unwrap();
    context.connection_revision = store
        .finish_revocation(&context, &intent)
        .unwrap()
        .connection_revision;
    (store, context, record)
}
#[test]
fn mcp_oauth_cleanup_claims_and_restart_preserve_tombstones_and_fence_superseded_sweeps() {
    use dunlin_mcp_client::oauth::persistence::CleanupJournal;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cleanup.db");
    let (store, context, record) = cleanup_ready(&path);
    let mut first = store.begin_cleanup(&context, 0).unwrap();
    assert_eq!(first.references, vec![record.secret_reference.clone()]);
    assert!(store.check_cleanup_target(&context, &first, 0).is_err());
    let id = first.operation_id.clone();
    first.operation_id.push('x');
    assert!(store.claim_cleanup_target(&context, &first, 0).is_err());
    first.operation_id = id;
    store.claim_cleanup_target(&context, &first, 0).unwrap();
    assert!(store.claim_cleanup_target(&context, &first, 0).is_err());
    store
        .acknowledge_cleanup_target(&context, &first, 0)
        .unwrap();
    assert_eq!(
        store
            .finish_cleanup(&context, &first)
            .unwrap()
            .delete_acknowledged,
        1
    );
    assert!(store.begin_cleanup(&context, 0).is_err());
    let second = store.begin_cleanup(&context, 1).unwrap();
    assert_eq!(second.references, first.references);
    store.claim_cleanup_target(&context, &second, 0).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let row = serde_json::to_value(
        store
            .mcp_oauth_connection(
                &context.user_id,
                &context.workspace_id,
                &context.connection_id,
            )
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(row["cleanup"]["status"], "unfinished");
    assert_eq!(row["cleanup"]["revision"], 2);
    let third = store.begin_cleanup(&context, 2).unwrap();
    assert_eq!(third.references, vec![record.secret_reference]);
    assert!(
        store
            .acknowledge_cleanup_target(&context, &second, 0)
            .is_err()
    );
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM mcp_oauth_secret_references",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert!(store.mcp_oauth_can_authorize(&context).is_ok());
}
#[test]
fn mcp_oauth_cleanup_protects_current_credentials_and_old_sweep_cannot_follow_new_authorization() {
    use dunlin_mcp_client::oauth::persistence::CleanupJournal;
    let dir = tempfile::tempdir().unwrap();
    let (store, mut context, old) = cleanup_ready(&dir.path().join("db"));
    let sweep = store.begin_cleanup(&context, 0).unwrap();
    let stale = context.clone();
    context.connection_revision = store
        .mcp_oauth_prepare(&context, BINDING, "fixture", None, &|_, _| Ok(()))
        .unwrap();
    let current = installed(&store, &context);
    assert_ne!(old.secret_reference, current.secret_reference);
    assert!(store.claim_cleanup_target(&stale, &sweep, 0).is_err());
    let fresh = store.begin_cleanup(&context, 1).unwrap();
    assert_eq!(fresh.references, vec![old.secret_reference.clone()]);
    store.claim_cleanup_target(&context, &fresh, 0).unwrap();
    // Even an imported database pointing at a retired record must protect its
    // current active reference; cleanup never treats the pointer as permission.
    store.connection.execute("UPDATE mcp_oauth_connections SET secret_reference=?2,generation=?3,sha256=?4 WHERE connection_id=?1",params![context.connection_id,old.secret_reference,old.generation as i64,old.sha256]).unwrap();
    assert!(store.check_cleanup_target(&context, &fresh, 0).is_err());
    assert!(
        store
            .acknowledge_cleanup_target(&context, &fresh, 0)
            .is_err()
    );
}
#[test]
fn mcp_oauth_cleanup_only_selects_exact_confirmed_envelopes() {
    use dunlin_mcp_client::oauth::persistence::{CleanupJournal, RevocationJournal};
    let dir = tempfile::tempdir().unwrap();
    let (store, mut context) = setup(&dir.path().join("db"));
    let first = installed(&store, &context);
    let refresh = store.begin_refresh(&context, BINDING, &first).unwrap();
    store
        .prepare_write(&context, BINDING, &refresh, DIGEST)
        .unwrap();
    store.commit(&context, BINDING, &refresh, DIGEST).unwrap();
    let intent = store.begin_revocation(&context, BINDING).unwrap();
    let index = intent
        .targets
        .iter()
        .position(|target| {
            target.as_ref().unwrap().record.secret_reference == first.secret_reference
        })
        .unwrap();
    store
        .claim_revocation_target(&context, &intent, index)
        .unwrap();
    store
        .confirm_revocation_target(&context, &intent, index)
        .unwrap();
    context.connection_revision = store
        .finish_revocation(&context, &intent)
        .unwrap()
        .connection_revision;
    let cleanup = store.begin_cleanup(&context, 0).unwrap();
    assert_eq!(cleanup.references, vec![first.secret_reference]);
}
#[test]
fn mcp_oauth_cleanup_migration44_preserves_active_credentials_and_rolls_back_collision() {
    for collision in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (store, context) = setup(&path);
        let original = installed(&store, &context);
        store
            .connection
            .execute_batch(
                "DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP INDEX IF EXISTS mcp_oauth_retention; ALTER TABLE mcp_oauth_secret_references DROP COLUMN delete_verified; ALTER TABLE mcp_oauth_secret_references DROP COLUMN retired; ALTER TABLE mcp_oauth_secret_references DROP COLUMN as_confirmed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN cleanup_attempted_revision; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_acknowledged; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_claimed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_operation_id; ALTER TABLE mcp_oauth_secret_references DROP COLUMN writer_protected; ALTER TABLE mcp_oauth_connections DROP COLUMN cleanup;PRAGMA user_version=43;",
            )
            .unwrap();
        if collision {
            store.connection.execute_batch("ALTER TABLE mcp_oauth_connections ADD COLUMN cleanup INTEGER;UPDATE mcp_oauth_connections SET cleanup=777;").unwrap();
        }
        drop(store);
        if collision {
            assert!(Store::open(&path).is_err());
            let raw = Connection::open(&path).unwrap();
            assert_eq!(
                raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                43
            );
            assert_eq!(
                raw.query_row("SELECT cleanup FROM mcp_oauth_connections", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                777
            );
            raw.execute_batch("ALTER TABLE mcp_oauth_connections DROP COLUMN cleanup;")
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.active(&context, BINDING).unwrap(), original);
        assert!(
            store
                .mcp_oauth_connection(
                    &context.user_id,
                    &context.workspace_id,
                    &context.connection_id
                )
                .unwrap()
                .unwrap()
                .cleanup
                .is_none()
        );
    }
}

#[test]
fn mcp_oauth_reconcile_requires_database_lease_and_preserves_unknown_after_restart() {
    use dunlin_mcp_client::oauth::persistence::RevocationJournal;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (store, context) = setup(&path);
    installed(&store, &context);
    let live = store.mcp_oauth_revocation_lease().unwrap();
    let intent = store.begin_revocation(&context, BINDING).unwrap();
    store.claim_revocation_target(&context, &intent, 0).unwrap();
    let current = AuthContext {
        connection_revision: intent.connection_revision,
        ..context.clone()
    };
    let other = Store::open(&path).unwrap();
    assert!(other.mcp_oauth_revocation_lease().is_err());
    assert!(
        other
            .mcp_oauth_reconcile_revocation(&current, &live, &|_, _| Ok(()))
            .is_err()
    );
    assert!(store.mcp_oauth_revocation_lease().is_err());
    drop(live);
    drop(store);
    let lease = other.mcp_oauth_revocation_lease().unwrap();
    let mut wrong = current.clone();
    wrong.user_id = "foreign".into();
    assert!(
        other
            .mcp_oauth_reconcile_revocation(&wrong, &lease, &|_, _| Ok(()))
            .is_err()
    );
    assert!(
        other
            .mcp_oauth_reconcile_revocation(&context, &lease, &|_, _| Ok(()))
            .is_err()
    );
    let result = other
        .mcp_oauth_reconcile_revocation(&current, &lease, &|_, _| Ok(()))
        .unwrap();
    assert_eq!((result.confirmed, result.unconfirmed), (0, 1));
    assert!(!result.confirmed_known_tokens);
    assert!(
        other
            .mcp_oauth_reconcile_revocation(&current, &lease, &|_, _| Ok(()))
            .is_err()
    );
    assert!(other.check_revocation_target(&context, &intent, 0).is_err());
    assert!(
        other
            .confirm_revocation_target(&context, &intent, 0)
            .is_err()
    );
    assert!(other.finish_revocation(&context, &intent).is_err());
    drop(lease);
    let row = serde_json::to_value(
        other
            .mcp_oauth_connection(
                &current.user_id,
                &current.workspace_id,
                &current.connection_id,
            )
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(row["status"], "disabled");
    assert_eq!(row["revocation"]["state"], "unconfirmed");
    assert_eq!(
        other
            .connection
            .query_row(
                "SELECT COUNT(*) FROM mcp_oauth_secret_references",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let authorized = other
        .mcp_oauth_prepare(&current, BINDING, "fixture", None, &|_, _| Ok(()))
        .unwrap();
    assert_eq!(authorized, 3);
}

#[test]
fn mcp_oauth_reconcile_keeps_confirmed_targets_and_does_not_invent_confirmation() {
    use dunlin_mcp_client::oauth::persistence::RevocationJournal;
    for acknowledged in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (store, context) = setup(&dir.path().join("db"));
        installed(&store, &context);
        let lease = store.mcp_oauth_revocation_lease().unwrap();
        let intent = store.begin_revocation(&context, BINDING).unwrap();
        if acknowledged {
            store.claim_revocation_target(&context, &intent, 0).unwrap();
            store
                .confirm_revocation_target(&context, &intent, 0)
                .unwrap();
        }
        drop(lease);
        let current = AuthContext {
            connection_revision: 2,
            ..context.clone()
        };
        let lease = store.mcp_oauth_revocation_lease().unwrap();
        let result = store
            .mcp_oauth_reconcile_revocation(&current, &lease, &|_, _| Ok(()))
            .unwrap();
        assert_eq!(result.confirmed_known_tokens, acknowledged);
        assert_eq!(result.confirmed, usize::from(acknowledged));
        assert_eq!(result.unconfirmed, usize::from(!acknowledged));
    }
}

#[test]
fn mcp_oauth_reconcile_rejects_legacy_unleased_operation_and_failed_fresh_authority() {
    use dunlin_mcp_client::oauth::persistence::RevocationJournal;
    for legacy in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (store, context) = setup(&dir.path().join("db"));
        installed(&store, &context);
        let lease = if legacy {
            None
        } else {
            Some(store.mcp_oauth_revocation_lease().unwrap())
        };
        store.begin_revocation(&context, BINDING).unwrap();
        drop(lease);
        let current = AuthContext {
            connection_revision: 2,
            ..context
        };
        let lease = store.mcp_oauth_revocation_lease().unwrap();
        let fail = |_: &Store, _: &AuthContext| Err(PersistenceError::Unauthorized);
        assert!(
            store
                .mcp_oauth_reconcile_revocation(
                    &current,
                    &lease,
                    if legacy { &|_, _| Ok(()) } else { &fail }
                )
                .is_err()
        );
        let row = serde_json::to_value(
            store
                .mcp_oauth_connection(
                    &current.user_id,
                    &current.workspace_id,
                    &current.connection_id,
                )
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(row["revocation"]["state"], "unfinished");
        if !legacy {
            assert!(
                store
                    .mcp_oauth_reconcile_revocation(&current, &lease, &|_, _| Ok(()))
                    .is_ok()
            );
        }
    }
}

#[test]
fn mcp_oauth_writer_lease_is_independent_of_disconnect_and_marks_exact_reserved_references() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (store, context) = setup(&path);
    let writer = store.mcp_oauth_writer_lease().unwrap();
    let first = installed(&store, &context);
    let other = Store::open(&path).unwrap();
    assert!(store.mcp_oauth_writer_lease().is_err());
    assert!(other.mcp_oauth_writer_lease().is_err());
    let negative = other.mcp_oauth_revocation_lease().unwrap();
    let refresh = store.begin_refresh(&context, BINDING, &first).unwrap();
    store
        .prepare_write(&context, BINDING, &refresh, DIGEST)
        .unwrap();
    assert_eq!(other.disable(&context).unwrap(), 2);
    assert!(store.commit(&context, BINDING, &refresh, DIGEST).is_err());
    assert!(other.mcp_oauth_writer_lease().is_err());
    drop(negative);
    drop(writer);
    let writer = other.mcp_oauth_writer_lease().unwrap();
    drop(writer);
    let protected: i64 = other
        .connection
        .query_row(
            "SELECT COUNT(*) FROM mcp_oauth_secret_references WHERE writer_protected=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(protected, 2);
    let context = AuthContext {
        connection_revision: 2,
        ..context
    };
    let third = other.begin_install(&context, BINDING).unwrap();
    assert_eq!(
        other
            .connection
            .query_row(
                "SELECT writer_protected FROM mcp_oauth_secret_references WHERE reference=?1",
                [third.staged_reference],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn mcp_oauth_writer_migration45_never_upgrades_legacy_proof_and_rolls_back_collision() {
    for collision in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (store, context) = setup(&path);
        let original = installed(&store, &context);
        store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP INDEX IF EXISTS mcp_oauth_retention; ALTER TABLE mcp_oauth_secret_references DROP COLUMN delete_verified; ALTER TABLE mcp_oauth_secret_references DROP COLUMN retired; ALTER TABLE mcp_oauth_secret_references DROP COLUMN as_confirmed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN cleanup_attempted_revision; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_acknowledged; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_claimed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_operation_id; ALTER TABLE mcp_oauth_secret_references DROP COLUMN writer_protected; PRAGMA user_version=44;").unwrap();
        if collision {
            store.connection.execute_batch("ALTER TABLE mcp_oauth_secret_references ADD COLUMN writer_protected TEXT; UPDATE mcp_oauth_secret_references SET writer_protected='preserve';").unwrap();
        }
        drop(store);
        if collision {
            assert!(Store::open(&path).is_err());
            let raw = Connection::open(&path).unwrap();
            assert_eq!(
                raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                44
            );
            assert_eq!(
                raw.query_row(
                    "SELECT writer_protected FROM mcp_oauth_secret_references",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "preserve"
            );
            raw.execute_batch(
                "ALTER TABLE mcp_oauth_secret_references DROP COLUMN writer_protected;",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.active(&context, BINDING).unwrap(), original);
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT writer_protected FROM mcp_oauth_secret_references",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        let writer = store.mcp_oauth_writer_lease().unwrap();
        let refreshing = store.begin_refresh(&context, BINDING, &original).unwrap();
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT writer_protected FROM mcp_oauth_secret_references WHERE reference=?1",
                    [refreshing.staged_reference],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        drop(writer);
    }
}

#[test]
fn mcp_oauth_write_receipt_is_exact_one_use_and_survives_negative_revision_and_auth_fence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (store, context) = setup(&path);
    let writer = store.mcp_oauth_writer_lease().unwrap();
    let intent = store.begin_install(&context, BINDING).unwrap();
    assert!(
        store
            .claim_write(&context, BINDING, &intent, DIGEST)
            .is_err()
    );
    store
        .prepare_write(&context, BINDING, &intent, DIGEST)
        .unwrap();
    assert!(
        store
            .acknowledge_write(&context, BINDING, &intent, DIGEST)
            .is_err()
    );
    let mut wrong = intent.clone();
    wrong.operation_id = "forged-operation".into();
    assert!(
        store
            .claim_write(&context, BINDING, &wrong, DIGEST)
            .is_err()
    );
    store
        .claim_write(&context, BINDING, &intent, DIGEST)
        .unwrap();
    assert!(
        store
            .claim_write(&context, BINDING, &intent, DIGEST)
            .is_err()
    );
    assert!(
        store
            .prepare_write(&context, BINDING, &intent, DIGEST)
            .is_err()
    );
    assert!(store.commit(&context, BINDING, &intent, DIGEST).is_err());
    assert_eq!(store.disable(&context).unwrap(), 2);
    {
        let denied = |_: &Store, _: &AuthContext| Err(PersistenceError::Unauthorized);
        let journal = store.mcp_oauth_journal(&denied);
        assert!(
            journal
                .claim_write(&context, BINDING, &intent, DIGEST)
                .is_err()
        );
        assert!(
            journal
                .acknowledge_write(&context, DIGEST, &intent, DIGEST)
                .is_err()
        );
        assert!(
            journal
                .acknowledge_write(&context, BINDING, &wrong, DIGEST)
                .is_err()
        );
        let mut foreign = context.clone();
        foreign.user_id = "foreign".into();
        assert!(
            journal
                .acknowledge_write(&foreign, BINDING, &intent, DIGEST)
                .is_err()
        );
        journal
            .acknowledge_write(&context, BINDING, &intent, DIGEST)
            .unwrap();
        assert!(journal.commit(&context, BINDING, &intent, DIGEST).is_err());
        assert!(
            store
                .acknowledge_write(&context, BINDING, &intent, DIGEST)
                .is_err()
        );
        assert_eq!(
            store.mcp_oauth_test_write_counts(&context.connection_id),
            (1, 1)
        );
    }
    drop(writer);
    drop(store);
    let store = Store::open(path).unwrap();
    assert_eq!(
        store.mcp_oauth_test_write_counts(&context.connection_id),
        (1, 1)
    );
    assert!(store.active(&context, BINDING).is_err());
    assert!(store.commit(&context, BINDING, &intent, DIGEST).is_err());
}

#[test]
fn mcp_oauth_write_receipt_restart_never_invents_ack_and_protected_commit_requires_it() {
    for phase in ["reserved", "prepared", "claimed", "acknowledged"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (store, context) = setup(&path);
        let writer = store.mcp_oauth_writer_lease().unwrap();
        let intent = store.begin_install(&context, BINDING).unwrap();
        if phase != "reserved" {
            store
                .prepare_write(&context, BINDING, &intent, DIGEST)
                .unwrap();
        }
        if ["claimed", "acknowledged"].contains(&phase) {
            store
                .claim_write(&context, BINDING, &intent, DIGEST)
                .unwrap();
        }
        if phase == "acknowledged" {
            store
                .acknowledge_write(&context, BINDING, &intent, DIGEST)
                .unwrap();
        }
        drop(writer);
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.mcp_oauth_test_write_counts(&context.connection_id),
            (
                i64::from(["claimed", "acknowledged"].contains(&phase)),
                i64::from(phase == "acknowledged")
            )
        );
        assert!(store.active(&context, BINDING).is_err());
        assert!(store.commit(&context, BINDING, &intent, DIGEST).is_err());
        let _writer = store.mcp_oauth_writer_lease().unwrap();
        assert_eq!(
            store.commit(&context, BINDING, &intent, DIGEST).is_ok(),
            phase == "acknowledged"
        );
    }
}

#[test]
fn mcp_oauth_write_receipt_migration46_preserves_legacy_unknown_and_rolls_back_last_collision() {
    for collision in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (store, context) = setup(&path);
        let record = installed(&store, &context);
        store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP INDEX IF EXISTS mcp_oauth_retention; ALTER TABLE mcp_oauth_secret_references DROP COLUMN delete_verified; ALTER TABLE mcp_oauth_secret_references DROP COLUMN retired; ALTER TABLE mcp_oauth_secret_references DROP COLUMN as_confirmed; ALTER TABLE mcp_oauth_secret_references DROP COLUMN cleanup_attempted_revision; ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_acknowledged;ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_claimed;ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_operation_id;PRAGMA user_version=45;").unwrap();
        if collision {
            store.connection.execute_batch("ALTER TABLE mcp_oauth_secret_references ADD COLUMN write_acknowledged TEXT;UPDATE mcp_oauth_secret_references SET write_acknowledged='preserve';").unwrap();
        }
        drop(store);
        if collision {
            assert!(Store::open(&path).is_err());
            let raw = Connection::open(&path).unwrap();
            assert_eq!(
                raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                45
            );
            assert_eq!(
                raw.query_row(
                    "SELECT write_acknowledged FROM mcp_oauth_secret_references",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "preserve"
            );
            let names = raw
                .prepare("PRAGMA table_info(mcp_oauth_secret_references)")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(
                !names
                    .iter()
                    .any(|n| n == "write_operation_id" || n == "write_claimed")
            );
            raw.execute_batch(
                "ALTER TABLE mcp_oauth_secret_references DROP COLUMN write_acknowledged;",
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.active(&context, BINDING).unwrap(), record);
        assert_eq!(
            store.mcp_oauth_test_write_counts(&context.connection_id),
            (0, 0)
        );
        let operation: Option<String> = store
            .connection
            .query_row(
                "SELECT write_operation_id FROM mcp_oauth_secret_references",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(operation.is_none());
    }
}

#[test]
fn mcp_oauth_write_receipt_ack_storage_failure_preserves_unknown_and_blocks_protected_commit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (store, context) = setup(&path);
    let writer = store.mcp_oauth_writer_lease().unwrap();
    let intent = store.begin_install(&context, BINDING).unwrap();
    store
        .prepare_write(&context, BINDING, &intent, DIGEST)
        .unwrap();
    store
        .claim_write(&context, BINDING, &intent, DIGEST)
        .unwrap();
    store.connection.execute_batch("CREATE TRIGGER fail_write_ack BEFORE UPDATE OF write_acknowledged ON mcp_oauth_secret_references WHEN NEW.write_acknowledged=1 BEGIN SELECT RAISE(ABORT,'injected write receipt failure');END;").unwrap();
    assert!(
        store
            .acknowledge_write(&context, BINDING, &intent, DIGEST)
            .is_err()
    );
    assert_eq!(
        store.mcp_oauth_test_write_counts(&context.connection_id),
        (1, 0)
    );
    assert!(store.commit(&context, BINDING, &intent, DIGEST).is_err());
    drop(writer);
    drop(store);
    let store = Store::open(path).unwrap();
    assert_eq!(
        store.mcp_oauth_test_write_counts(&context.connection_id),
        (1, 0)
    );
    assert!(store.active(&context, BINDING).is_err());
    assert!(store.commit(&context, BINDING, &intent, DIGEST).is_err());
    assert!(
        store
            .claim_write(&context, BINDING, &intent, DIGEST)
            .is_err()
    );
}

fn retention_fill(
    store: &Store,
    c: &AuthContext,
    count: usize,
    protected: bool,
) -> CredentialRecord {
    let _writer = if protected {
        Some(store.mcp_oauth_writer_lease().unwrap())
    } else {
        None
    };
    let mut record = installed(store, c);
    for _ in 1..count {
        let intent = store.begin_refresh(c, BINDING, &record).unwrap();
        store.prepare_write(c, BINDING, &intent, DIGEST).unwrap();
        store.claim_write(c, BINDING, &intent, DIGEST).unwrap();
        store
            .acknowledge_write(c, BINDING, &intent, DIGEST)
            .unwrap();
        store.commit(c, BINDING, &intent, DIGEST).unwrap();
        record = store.active(c, BINDING).unwrap();
    }
    record
}
fn retention_revoke(store: &Store, c: &mut AuthContext) -> usize {
    use dunlin_mcp_client::oauth::persistence::RevocationJournal;
    let intent = store.begin_revocation(c, BINDING).unwrap();
    let count = intent.targets.len();
    for n in 0..count {
        if !intent.already_confirmed[n] {
            store.claim_revocation_target(c, &intent, n).unwrap();
            store.confirm_revocation_target(c, &intent, n).unwrap();
        }
    }
    c.connection_revision = store
        .finish_revocation(c, &intent)
        .unwrap()
        .connection_revision;
    count
}
fn retention_sweep(
    store: &Store,
    c: &AuthContext,
    expected: u64,
) -> dunlin_mcp_client::oauth::persistence::CleanupSummary {
    use dunlin_mcp_client::oauth::persistence::CleanupJournal;
    let _writer = store.mcp_oauth_writer_lease().unwrap();
    let sweep = store.begin_cleanup(c, expected).unwrap();
    assert!(sweep.references.len() <= 256);
    for n in 0..sweep.references.len() {
        store.claim_cleanup_target(c, &sweep, n).unwrap();
        store.check_cleanup_target(c, &sweep, n).unwrap();
        store.acknowledge_cleanup_target(c, &sweep, n).unwrap();
    }
    store.finish_cleanup(c, &sweep).unwrap()
}

#[test]
fn mcp_oauth_retention_reclaims_active_quota_beyond_256_and_bounds_history_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (mut store, mut c) = setup(&path);
    let mut sequence = 0;
    let mut pruned = 0;
    for cycle in 0..3 {
        if cycle > 0 {
            c.connection_revision = store
                .mcp_oauth_prepare(&c, BINDING, "fixture", None, &|_, _| Ok(()))
                .unwrap();
        }
        let record = retention_fill(&store, &c, 256, true);
        assert!(store.begin_refresh(&c, BINDING, &record).is_err());
        assert_eq!(
            retention::storage(&store.connection, &c.connection_id)
                .unwrap()
                .available_slots,
            0
        );
        // Only this active batch enters a new AS journal; prior history never
        // requires sending its known-revoked tokens again.
        assert_eq!(retention_revoke(&store, &mut c), 256);
        let mut recovered = 0;
        for _ in 0..4 {
            let summary = retention_sweep(&store, &c, sequence);
            sequence = summary.revision;
            recovered += summary.reclaimed;
            pruned += summary.pruned;
            assert_eq!(summary.unconfirmed, 0);
            assert!(
                store
                    .connection
                    .query_row(
                        "SELECT COUNT(*) FROM mcp_oauth_secret_references",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap()
                    <= retention::MAX_REFERENCES as i64
            );
            drop(store);
            store = Store::open(&path).unwrap();
            if retention::storage(&store.connection, &c.connection_id)
                .unwrap()
                .pending_references
                == 0
            {
                break;
            }
        }
        assert_eq!(recovered, 256);
        let storage = retention::storage(&store.connection, &c.connection_id).unwrap();
        assert_eq!(storage.available_slots, 256);
        assert_eq!(storage.uncertain_history, 0);
        assert!(storage.retained_history <= 512);
    }
    assert_eq!(pruned, 256);
    assert_eq!(
        retention::storage(&store.connection, &c.connection_id)
            .unwrap()
            .retained_history,
        512
    );
    // Imported current pointers are protected even if they name otherwise
    // eligible old history. The oldest *other* verified row must expire.
    let protected: (String,i64,String)=store.connection.query_row("SELECT reference,generation,sha256 FROM mcp_oauth_secret_references ORDER BY created_at,reference LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    store.connection.execute("INSERT INTO mcp_oauth_secret_references(reference,connection_id,created_at,credential_revision,binding,generation,sha256,writer_protected,write_operation_id,write_claimed,write_acknowledged,as_confirmed,retired,delete_verified,cleanup_attempted_revision) SELECT ?1,connection_id,created_at+1,credential_revision,binding,generation,sha256,1,?2,1,1,1,1,1,cleanup_attempted_revision FROM mcp_oauth_secret_references LIMIT 1",params![format!("mcp-oauth:{}",uuid::Uuid::now_v7()),uuid::Uuid::now_v7().to_string()]).unwrap();
    store.connection.execute("UPDATE mcp_oauth_connections SET state='active',secret_reference=?2,generation=?3,sha256=?4 WHERE connection_id=?1",params![c.connection_id,protected.0,protected.1,protected.2]).unwrap();
    let _writer = store.mcp_oauth_writer_lease().unwrap();
    assert_eq!(retention::prune(&store.connection, &c, true).unwrap(), 1);
    assert!(
        store
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM mcp_oauth_secret_references WHERE reference=?1)",
                [protected.0],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
}

#[test]
fn mcp_oauth_retention_as_confirmations_survive_missing_journal_and_registration_change() {
    use dunlin_mcp_client::oauth::persistence::{CleanupJournal, RevocationJournal};
    let dir = tempfile::tempdir().unwrap();
    let (store, mut c) = setup(&dir.path().join("db"));
    let old = retention_fill(&store, &c, 1, true);
    retention_revoke(&store, &mut c);
    store
        .connection
        .execute(
            "UPDATE mcp_oauth_connections SET revocation=NULL WHERE connection_id=?1",
            [&c.connection_id],
        )
        .unwrap();
    assert!(
        retention::storage(&store.connection, &c.connection_id)
            .unwrap()
            .cleanup_available
    );
    c.connection_revision = store
        .mcp_oauth_prepare(&c, DIGEST, "changed", None, &|_, _| Ok(()))
        .unwrap();
    {
        let _writer = store.mcp_oauth_writer_lease().unwrap();
        let i = store.begin_install(&c, DIGEST).unwrap();
        store.prepare_write(&c, DIGEST, &i, DIGEST).unwrap();
        store.claim_write(&c, DIGEST, &i, DIGEST).unwrap();
        store.acknowledge_write(&c, DIGEST, &i, DIGEST).unwrap();
        store.commit(&c, DIGEST, &i, DIGEST).unwrap();
    }
    let intent = store.begin_revocation(&c, DIGEST).unwrap();
    assert_eq!(intent.targets.len(), 2);
    assert_eq!(intent.already_confirmed.iter().filter(|b| **b).count(), 1);
    let new = intent.already_confirmed.iter().position(|b| !*b).unwrap();
    let acknowledged = 1 - new;
    assert!(
        intent.targets[acknowledged].is_none(),
        "old registration is acknowledged without materializing its token"
    );
    store.claim_revocation_target(&c, &intent, new).unwrap();
    store.confirm_revocation_target(&c, &intent, new).unwrap();
    c.connection_revision = store
        .finish_revocation(&c, &intent)
        .unwrap()
        .connection_revision;
    let _writer = store.mcp_oauth_writer_lease().unwrap();
    let sweep = store.begin_cleanup(&c, 0).unwrap();
    assert_eq!(sweep.references.len(), 2);
    assert!(sweep.references.contains(&old.secret_reference));
}

#[test]
fn mcp_oauth_retention_uncertain_history_is_bounded_and_not_silently_pruned() {
    use dunlin_mcp_client::oauth::persistence::RevocationJournal;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (store, mut c) = setup(&path);
    retention_fill(&store, &c, 256, false);
    retention_revoke(&store, &mut c);
    let first = retention_sweep(&store, &c, 0);
    assert_eq!(first.reclaimed, 256);
    assert_eq!(first.pruned, 0);
    assert_eq!(
        retention::storage(&store.connection, &c.connection_id)
            .unwrap()
            .uncertain_history,
        256
    );
    c.connection_revision = store
        .mcp_oauth_prepare(&c, BINDING, "fixture", None, &|_, _| Ok(()))
        .unwrap();
    {
        let _writer = store.mcp_oauth_writer_lease().unwrap();
        let i = store.begin_install(&c, BINDING).unwrap();
        store.prepare_write(&c, BINDING, &i, DIGEST).unwrap();
        store.claim_write(&c, BINDING, &i, DIGEST).unwrap();
    }
    let intent = store.begin_revocation(&c, BINDING).unwrap();
    assert_eq!(intent.targets.len(), 1);
    store.claim_revocation_target(&c, &intent, 0).unwrap();
    store.confirm_revocation_target(&c, &intent, 0).unwrap();
    c.connection_revision = store
        .finish_revocation(&c, &intent)
        .unwrap()
        .connection_revision;
    let second = retention_sweep(&store, &c, first.revision);
    let third = retention_sweep(&store, &c, second.revision);
    assert_eq!(second.reclaimed + third.reclaimed, 0);
    assert_eq!(second.pruned + third.pruned, 0);
    drop(store);
    let store = Store::open(path).unwrap();
    let storage = retention::storage(&store.connection, &c.connection_id).unwrap();
    assert_eq!(storage.pending_references, 1);
    assert_eq!(storage.uncertain_history, 256);
    assert_eq!(storage.available_slots, 255);
}

#[test]
fn mcp_oauth_retention_migration47_preserves_records_and_rolls_back_late_collisions() {
    for collision in ["none", "column", "index"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let (store, c) = setup(&path);
        let record = retention_fill(&store, &c, 1, true);
        store.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; DROP INDEX mcp_oauth_retention;ALTER TABLE mcp_oauth_secret_references DROP COLUMN delete_verified;ALTER TABLE mcp_oauth_secret_references DROP COLUMN retired;ALTER TABLE mcp_oauth_secret_references DROP COLUMN as_confirmed;ALTER TABLE mcp_oauth_secret_references DROP COLUMN cleanup_attempted_revision;PRAGMA user_version=46;").unwrap();
        if collision == "column" {
            store.connection.execute_batch("ALTER TABLE mcp_oauth_secret_references ADD COLUMN cleanup_attempted_revision TEXT;UPDATE mcp_oauth_secret_references SET cleanup_attempted_revision='preserve';").unwrap();
        }
        if collision == "index" {
            store.connection.execute_batch("CREATE TABLE retention_collision(value TEXT);CREATE INDEX mcp_oauth_retention ON retention_collision(value);").unwrap();
        }
        drop(store);
        if collision != "none" {
            assert!(Store::open(&path).is_err());
            let raw = Connection::open(&path).unwrap();
            assert_eq!(
                raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                46
            );
            let names = raw
                .prepare("PRAGMA table_info(mcp_oauth_secret_references)")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(
                !names
                    .iter()
                    .any(|n| n == "as_confirmed" || n == "retired" || n == "delete_verified")
            );
            if collision == "column" {
                assert_eq!(
                    raw.query_row(
                        "SELECT cleanup_attempted_revision FROM mcp_oauth_secret_references",
                        [],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                    "preserve"
                );
                raw.execute_batch("ALTER TABLE mcp_oauth_secret_references DROP COLUMN cleanup_attempted_revision;").unwrap();
            } else {
                raw.execute_batch("DROP INDEX mcp_oauth_retention;DROP TABLE retention_collision;")
                    .unwrap();
            }
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.active(&c, BINDING).unwrap(), record);
        assert_eq!(store.mcp_oauth_test_write_counts(&c.connection_id), (1, 1));
        let storage = retention::storage(&store.connection, &c.connection_id).unwrap();
        assert_eq!(storage.pending_references, 1);
        assert_eq!(storage.retained_history, 0);
        assert!(!storage.cleanup_available);
    }
}
