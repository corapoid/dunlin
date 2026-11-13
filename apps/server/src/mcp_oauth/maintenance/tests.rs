use super::*;
use dunlin_domain::collaboration::WorkspaceRole;
use dunlin_mcp_client::oauth::{
    AuthContext,
    persistence::{CredentialJournal, RevocationIntent, RevocationJournal},
};
use rusqlite::{Connection, params};
const BINDING: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
struct Fixture {
    dir: tempfile::TempDir,
    path: PathBuf,
    store: Store,
    context: AuthContext,
    actor: String,
    intent: RevocationIntent,
}
impl Fixture {
    fn new(confirmed: usize) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("relay.db");
        let store = Store::open(&path).unwrap();
        let owner = store.create_user("legacy-owner", "admin").unwrap();
        let actor = store.create_user("legacy-operator", "admin").unwrap();
        let team = store.create_workspace(&owner, "Legacy fixture").unwrap();
        store
            .set_team_member(
                &owner.id,
                &team.id,
                &actor.id,
                WorkspaceRole::Admin,
                true,
                None,
            )
            .unwrap();
        let context = AuthContext {
            user_id: owner.id,
            workspace_id: team.id,
            connection_id: uuid::Uuid::now_v7().to_string(),
            connection_revision: 1,
        };
        for generation in 0..2 {
            let intent = if generation == 0 {
                store.begin_install(&context, BINDING).unwrap()
            } else {
                store
                    .begin_refresh(&context, BINDING, &store.active(&context, BINDING).unwrap())
                    .unwrap()
            };
            store
                .prepare_write(&context, BINDING, &intent, DIGEST)
                .unwrap();
            store
                .claim_write(&context, BINDING, &intent, DIGEST)
                .unwrap();
            store
                .acknowledge_write(&context, BINDING, &intent, DIGEST)
                .unwrap();
            store.commit(&context, BINDING, &intent, DIGEST).unwrap();
        }
        // No worker lease: this is synthetic pre-lease metadata, not a live AS job.
        let intent = store.begin_revocation(&context, BINDING).unwrap();
        for n in 0..2 {
            store.claim_revocation_target(&context, &intent, n).unwrap();
            if n < confirmed {
                store
                    .confirm_revocation_target(&context, &intent, n)
                    .unwrap();
            }
        }
        Self {
            dir,
            path,
            store,
            context,
            actor: actor.id,
            intent,
        }
    }
    fn args(&self, action: &str) -> Vec<String> {
        vec![
            action.into(),
            "--db".into(),
            self.path.to_str().unwrap().into(),
            "--workspace-id".into(),
            self.context.workspace_id.clone(),
            "--connection-id".into(),
            self.context.connection_id.clone(),
            "--actor-user-id".into(),
            self.actor.clone(),
        ]
    }
    fn review(&self) -> Value {
        serde_json::from_str(&run(&self.args("show")).unwrap()).unwrap()
    }
    fn recovery(&self, review: &Value) -> Vec<String> {
        let mut args = self.args("recover");
        args.extend([
            "--expected-revision".into(),
            review["review"]["revision"].to_string(),
            "--review-sha256".into(),
            review["review"]["review_sha256"].as_str().unwrap().into(),
            "--change-id".into(),
            "OPS-legacy-1".into(),
            "--ack-all-legacy-relays-stopped".into(),
        ]);
        args
    }
    fn db(&self) -> Connection {
        Connection::open(&self.path).unwrap()
    }
    fn ledger(&self) -> Value {
        serde_json::from_str(
            &self
                .db()
                .query_row(
                    "SELECT revocation FROM mcp_oauth_connections WHERE connection_id=?1",
                    [&self.context.connection_id],
                    |r| r.get::<_, String>(0),
                )
                .unwrap(),
        )
        .unwrap()
    }
    fn references(&self) -> Vec<Vec<String>> {
        let db = self.db();
        let mut q = db
            .prepare("SELECT * FROM mcp_oauth_secret_references ORDER BY reference")
            .unwrap();
        q.query_map([], |r| {
            Ok((0..r.as_ref().column_count())
                .map(|n| format!("{:?}", r.get_ref(n).unwrap()))
                .collect())
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }
}
#[test]
fn legacy_operator_parser_requires_explicit_review_and_shutdown_without_implicit_defaults() {
    let f = Fixture::new(0);
    let show = f.args("show");
    parse(&show).unwrap();
    for extra in [
        vec!["--force"],
        vec!["extra"],
        vec!["--db", "/other"],
        vec!["--ack-all-legacy-relays-stopped"],
    ] {
        let mut a = show.clone();
        a.extend(extra.into_iter().map(str::to_owned));
        assert!(parse(&a).is_err());
    }
    let review = f.review();
    let complete = f.recovery(&review);
    parse(&complete).unwrap();
    for name in [
        "--db",
        "--expected-revision",
        "--review-sha256",
        "--change-id",
        "--actor-user-id",
        "--ack-all-legacy-relays-stopped",
    ] {
        let mut a = complete.clone();
        let n = a.iter().position(|v| v == name).unwrap();
        a.remove(n);
        if name != "--ack-all-legacy-relays-stopped" {
            a.remove(n);
        }
        assert!(parse(&a).is_err(), "{name}");
    }
    for bad in ["0", "-1", "+2", "9007199254740991", "9007199254740992"] {
        let mut a = complete.clone();
        let n = a.iter().position(|v| v == "--expected-revision").unwrap();
        a[n + 1] = bad.into();
        assert!(parse(&a).is_err());
    }
}
#[test]
fn legacy_operator_recovery_preserves_unknowns_fences_old_intent_and_carries_provenance() {
    for confirmed in [0, 1, 2] {
        let f = Fixture::new(confirmed);
        let before = f.ledger();
        let refs = f.references();
        let review = f.review();
        assert_eq!(before, f.ledger());
        assert_eq!(review["review"]["confirmed"], confirmed);
        assert_eq!(review["review"]["unconfirmed"], 2 - confirmed);
        let encoded = run(&f.recovery(&review)).unwrap();
        let result: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(result["revision"], 3);
        assert_eq!(result["confirmed"], confirmed);
        assert_eq!(result["unconfirmed"], 2 - confirmed);
        assert_eq!(result["new_confirmations"], 0);
        assert_eq!(result["local_stop_proof"], "operator_attestation");
        assert_eq!(refs, f.references());
        let after = f.ledger();
        assert_eq!(after["targets"], before["targets"]);
        assert_eq!(after["lease_protected"], false);
        assert_ne!(after["id"], before["id"]);
        assert_eq!(after["operator_recovery"]["original_source_revision"], 1);
        assert!(run(&f.recovery(&review)).is_err());
        for n in 0..2 {
            assert!(
                f.store
                    .check_revocation_target(&f.context, &f.intent, n)
                    .is_err()
            );
            assert!(
                f.store
                    .confirm_revocation_target(&f.context, &f.intent, n)
                    .is_err()
            );
        }
        assert!(f.store.finish_revocation(&f.context, &f.intent).is_err());
        let current = AuthContext {
            connection_revision: 3,
            ..f.context.clone()
        };
        assert!(f.store.mcp_oauth_can_authorize(&current).is_ok());
        let public = serde_json::to_string(
            &f.store
                .mcp_oauth_connection(
                    &f.context.user_id,
                    &f.context.workspace_id,
                    &f.context.connection_id,
                )
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(!public.contains("OPS-legacy-1"));
        assert!(!public.contains(review["review"]["review_sha256"].as_str().unwrap()));
        if confirmed < 2 {
            let _lease = f.store.mcp_oauth_revocation_lease().unwrap();
            f.store.begin_revocation(&current, BINDING).unwrap();
            assert_eq!(f.ledger()["operator_recovery"], after["operator_recovery"]);
        }
        for secret in [BINDING, DIGEST, f.intent.operation_id.as_str()] {
            assert!(!encoded.contains(secret));
        }
    }
}
#[test]
fn legacy_operator_recovery_rejects_stale_review_authority_and_both_live_modern_workers() {
    let f = Fixture::new(1);
    let review = f.review();
    let args = f.recovery(&review);
    let before = f.ledger();
    let lease = f.store.mcp_oauth_revocation_lease().unwrap();
    assert!(run(&args).is_err());
    drop(lease);
    assert_eq!(before, f.ledger());
    let lease = f.store.mcp_oauth_writer_lease().unwrap();
    assert!(run(&args).is_err());
    drop(lease);
    assert_eq!(before, f.ledger());
    f.db()
        .execute(
            "UPDATE team_memberships SET role='member',revision=revision+1 WHERE user_id=?1",
            [&f.actor],
        )
        .unwrap();
    assert!(run(&args).is_err());
    f.db()
        .execute(
            "UPDATE team_memberships SET role='admin',revision=revision+1 WHERE user_id=?1",
            [&f.actor],
        )
        .unwrap();
    assert!(run(&args).is_err());
    assert_eq!(before, f.ledger());
    let fresh = f.review();
    assert_ne!(
        review["review"]["review_sha256"],
        fresh["review"]["review_sha256"]
    );
    // A legitimate acknowledgement from the old worker changes the exact
    // reviewed journal even though its connection revision is unchanged.
    f.store
        .confirm_revocation_target(&f.context, &f.intent, 1)
        .unwrap();
    assert!(run(&f.recovery(&fresh)).is_err());
    run(&f.recovery(&f.review())).unwrap();
}
#[test]
fn legacy_operator_recovery_does_not_reactivate_removed_owner_and_rolls_back_audit_failure() {
    let f = Fixture::new(1);
    f.db()
        .execute(
            "UPDATE team_memberships SET active=0,revision=revision+1 WHERE user_id=?1",
            [&f.context.user_id],
        )
        .unwrap();
    let review = f.review();
    let before = f.ledger();
    let refs = f.references();
    f.db().execute_batch("CREATE TRIGGER fail_legacy_audit BEFORE INSERT ON team_audit WHEN NEW.action='mcp_oauth_legacy_operator_recovery' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(run(&f.recovery(&review)).is_err());
    assert_eq!(before, f.ledger());
    assert_eq!(refs, f.references());
    f.db()
        .execute_batch("DROP TRIGGER fail_legacy_audit;")
        .unwrap();
    run(&f.recovery(&review)).unwrap();
    assert!(
        f.store
            .mcp_oauth_can_authorize(&AuthContext {
                connection_revision: 3,
                ..f.context.clone()
            })
            .is_err()
    );
    let actor: String = f
        .db()
        .query_row(
            "SELECT actor_id FROM team_audit WHERE action='mcp_oauth_legacy_operator_recovery'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(actor, f.actor);
}
#[test]
fn legacy_operator_review_is_bound_to_database_identity_and_never_creates_or_upgrades_files() {
    let f = Fixture::new(0);
    let review = f.review();
    let before = f.ledger();
    let missing = f.dir.path().join("missing.db");
    let mut args = f.args("show");
    args[2] = missing.to_str().unwrap().into();
    assert!(run(&args).is_err());
    assert!(!missing.exists());
    let bogus = f.dir.path().join("bogus.db");
    std::fs::write(&bogus, b"non-sqlite operator file").unwrap();
    args[2] = bogus.to_str().unwrap().into();
    assert!(run(&args).is_err());
    assert_eq!(std::fs::read(&bogus).unwrap(), b"non-sqlite operator file");
    for version in [
        crate::store::SCHEMA_VERSION - 1,
        crate::store::SCHEMA_VERSION + 1,
    ] {
        f.db().pragma_update(None, "user_version", version).unwrap();
        assert!(run(&f.args("show")).is_err());
        assert_eq!(
            f.db()
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
    }
    f.db()
        .pragma_update(None, "user_version", crate::store::SCHEMA_VERSION)
        .unwrap();
    f.db()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    let other = f.dir.path().join("copy.db");
    std::fs::copy(&f.path, &other).unwrap();
    let mut cloned = f.recovery(&review);
    cloned[2] = other.to_str().unwrap().into();
    assert!(run(&cloned).is_err());
    assert_eq!(before, f.ledger());
    let _guard = f.store.mcp_oauth_revocation_lease().unwrap();
    let modern = f
        .store
        .mcp_oauth_review_legacy(&f.actor, &f.context.workspace_id, &f.context.connection_id)
        .unwrap();
    assert_eq!(
        modern.review_sha256,
        review["review"]["review_sha256"].as_str().unwrap()
    );
    drop(_guard);
    f.db().execute("UPDATE mcp_oauth_connections SET revocation=json_set(revocation,'$.lease_protected',json('true')) WHERE connection_id=?1",[&f.context.connection_id]).unwrap();
    assert!(run(&f.args("show")).is_err());
    assert!(run(&f.recovery(&review)).is_err());
}
#[cfg(unix)]
#[test]
fn legacy_operator_preflight_rejects_symlink_hardlink_fifo_and_shared_writes() {
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{PermissionsExt, symlink},
    };
    let f = Fixture::new(0);
    let mut args = f.args("show");
    let alias = f.dir.path().join("alias.db");
    symlink(&f.path, &alias).unwrap();
    args[2] = alias.to_str().unwrap().into();
    assert!(run(&args).is_err());
    std::fs::remove_file(&alias).unwrap();
    std::fs::hard_link(&f.path, &alias).unwrap();
    assert!(run(&args).is_err());
    assert!(run(&f.args("show")).is_err());
    std::fs::remove_file(&alias).unwrap();
    let name = std::ffi::CString::new(alias.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(run(&args).is_err());
    std::fs::set_permissions(&f.path, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(run(&f.args("show")).is_err());
}
#[test]
fn legacy_operator_full_audit_keeps_bounded_durable_provenance() {
    let f = Fixture::new(1);
    let db = f.db();
    db.execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000-(SELECT count(*) FROM team_audit)) INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at) SELECT ?1,?2,?2,'fixture',1,1 FROM n",params![f.context.workspace_id,f.actor]).unwrap();
    let count = || {
        db.query_row("SELECT count(*) FROM team_audit", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
    };
    assert_eq!(count(), 100000);
    run(&f.recovery(&f.review())).unwrap();
    assert_eq!(count(), 100000);
    assert_eq!(f.ledger()["operator_recovery"]["change_id"], "OPS-legacy-1");
}
#[cfg(unix)]
#[test]
#[ignore = "requires production executable in the runtime sandbox"]
fn production_browser_acceptance_runtime_mcp_oauth_legacy_recovery() {
    use std::process::Command;
    let bin = std::env::var_os("DUNLIN_RELAY_E2E_BIN").expect("production executable required");
    let f = Fixture::new(1);
    let before = f.references();
    let invoke = |args: &[String]| {
        Command::new(&bin)
            .arg("mcp-oauth-legacy")
            .args(args)
            .env_clear()
            .output()
            .unwrap()
    };
    let shown = invoke(&f.args("show"));
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let review: Value = serde_json::from_slice(&shown.stdout).unwrap();
    let args = f.recovery(&review);
    let guard = f.store.mcp_oauth_writer_lease().unwrap();
    assert!(!invoke(&args).status.success());
    drop(guard);
    let done = invoke(&args);
    assert!(
        done.status.success(),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    let result: Value = serde_json::from_slice(&done.stdout).unwrap();
    assert_eq!(result["revision"], 3);
    assert_eq!(result["unconfirmed"], 1);
    assert_eq!(before, f.references());
    assert!(!invoke(&args).status.success());
    let text = String::from_utf8(done.stdout).unwrap();
    for private in [BINDING, DIGEST, f.intent.operation_id.as_str()] {
        assert!(!text.contains(private));
    }
}
