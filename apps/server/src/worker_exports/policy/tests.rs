use super::*;
use crate::worker_exports::{
    CapturedCheckout, ExportPermission, ExportPrincipal, ExportScope, PrincipalKind, WorkerStore,
};
const RAW: &[u8] = br#"{"schema_version":1,"harnesses":["codex"],"sandbox_required":true,"credential_mode":"isolated","max_seconds":60,"max_output_bytes":65536,"max_parallel":1}
"#;
#[test]
fn broker_policy_requires_explicit_schema2_opt_in_and_preserves_legacy_bytes() {
    let legacy = PolicySnapshot::parse(RAW.to_vec()).unwrap();
    assert!(!legacy.permits_mcp_broker().unwrap());
    let mut value: serde_json::Value = serde_json::from_slice(RAW).unwrap();
    for version in [1, 2] {
        for flag in [None, Some(false), Some(true)] {
            value["schema_version"] = serde_json::json!(version);
            value.as_object_mut().unwrap().remove("mcp_broker");
            if let Some(flag) = flag {
                value["mcp_broker"] = serde_json::json!(flag);
            }
            let raw = serde_json::to_vec(&value).unwrap();
            let parsed = PolicySnapshot::parse(raw.clone());
            if (version == 1 && flag.is_none()) || (version == 2 && flag == Some(true)) {
                let parsed = parsed.unwrap();
                assert_eq!(parsed.permits_mcp_broker().unwrap(), version == 2);
                assert_eq!(parsed.digest(), hex::encode(Sha256::digest(raw)));
            } else {
                assert!(parsed.is_err());
            }
        }
    }
    for version in [1, 2] {
        let mut value: serde_json::Value = serde_json::from_slice(RAW).unwrap();
        value["schema_version"] = serde_json::json!(version);
        value["mcp_broker"] = serde_json::Value::Null;
        assert!(PolicySnapshot::parse(serde_json::to_vec(&value).unwrap()).is_err());
    }
    assert_eq!(legacy.raw, RAW);
}
struct Fixture {
    dir: tempfile::TempDir,
    scope: ExportScope,
}
impl Fixture {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            scope: ExportScope {
                controller_node_id: "controller".into(),
                worker_node_id: "worker".into(),
                workspace_id: "workspace".into(),
                host_id: "host".into(),
                enrollment_epoch: 1,
                credential_revision: 1,
            },
        }
    }
    fn store(&self) -> WorkerStore {
        WorkerStore::open(&self.dir.path().join("state"), self.scope.clone()).unwrap()
    }
    fn checkout(&self, digest: &str) -> CapturedCheckout {
        let root = self.dir.path().join("checkout");
        std::fs::create_dir_all(&root).unwrap();
        CapturedCheckout::capture(&root, digest, None).unwrap()
    }
}
fn principal() -> ExportPrincipal {
    ExportPrincipal {
        kind: PrincipalKind::Human,
        id: "human".into(),
    }
}
#[test]
fn raw_hash_preserves_every_byte_and_strict_policy_rejects_extensions() {
    let policy = PolicySnapshot::parse(RAW.to_vec()).unwrap();
    assert_eq!(policy.digest(), hex::encode(Sha256::digest(RAW)));
    let trimmed = PolicySnapshot::parse(RAW[..RAW.len() - 1].to_vec()).unwrap();
    assert_ne!(trimmed.digest(), policy.digest());
    for (field, value) in [
        ("schema_version", serde_json::json!(3)),
        ("sandbox_required", serde_json::json!(false)),
        ("credential_mode", serde_json::json!("inherited")),
        ("harnesses", serde_json::json!(["codex", "codex"])),
        ("harnesses", serde_json::json!(["shell"])),
        ("max_parallel", serde_json::json!(9)),
        ("max_seconds", serde_json::json!(0)),
        ("max_output_bytes", serde_json::json!(16777217)),
        ("env", serde_json::json!({})),
        ("extra_roots", serde_json::json!([])),
    ] {
        let mut value_object: serde_json::Value = serde_json::from_slice(RAW).unwrap();
        value_object[field] = value;
        assert!(
            PolicySnapshot::parse(serde_json::to_vec(&value_object).unwrap()).is_err(),
            "{field}"
        );
    }
    assert!(PolicySnapshot::parse(vec![b' '; MAX_BYTES + 1]).is_err());
    let mut bom = vec![0xef, 0xbb, 0xbf];
    bom.extend_from_slice(RAW);
    assert!(PolicySnapshot::parse(bom).is_err());
}
#[test]
fn snapshot_reopens_immutable_and_metadata_rebind_drops_policy_and_grants() {
    let f = Fixture::new();
    let policy = PolicySnapshot::parse(RAW.to_vec()).unwrap();
    let store = f.store();
    let checkout = f.checkout(policy.digest());
    let export = store
        .register_captured_with_policy(&f.scope, "Policy", &checkout, Some(&policy))
        .unwrap();
    store
        .set_grant(
            &f.scope,
            &export.id,
            &principal(),
            &[ExportPermission::Read, ExportPermission::Execute],
            true,
            None,
        )
        .unwrap();
    drop(store);
    let store = f.store();
    assert_eq!(
        store
            .policy_snapshot(&export.id, policy.digest())
            .unwrap()
            .unwrap()
            .raw,
        RAW
    );
    let authority = store
        .authorize_validated(
            &f.scope,
            store
                .candidate(
                    &f.scope,
                    &export.id,
                    &principal(),
                    ExportPermission::Execute,
                )
                .unwrap()
                .validate_filesystem()
                .unwrap(),
        )
        .unwrap();
    let plain = f.checkout(&"a".repeat(64));
    let changed = store
        .rebind_captured(&f.scope, &export.id, 1, "Metadata only", &plain)
        .unwrap();
    assert_eq!(changed.revision, 2);
    assert!(
        store
            .policy_snapshot(&export.id, plain.settings_digest())
            .unwrap()
            .is_none()
    );
    assert!(!store.grants(&f.scope, &export.id).unwrap()[0].active);
    assert!(store.revalidate_authority(&f.scope, &authority).is_err());
    assert!(
        store
            .rebind_captured_with_policy(&f.scope, &export.id, 1, "Stale", &checkout, Some(&policy))
            .is_err()
    );
}
#[test]
fn late_policy_failure_rolls_back_export_rebind_and_grant_invalidation() {
    let f = Fixture::new();
    let store = f.store();
    let policy = PolicySnapshot::parse(RAW.to_vec()).unwrap();
    let checkout = f.checkout(policy.digest());
    let export = store
        .register_captured_with_policy(&f.scope, "Policy", &checkout, Some(&policy))
        .unwrap();
    store
        .set_grant(
            &f.scope,
            &export.id,
            &principal(),
            &[ExportPermission::Read],
            true,
            None,
        )
        .unwrap();
    store.connection.execute_batch("CREATE TRIGGER reject_policy BEFORE INSERT ON export_execution_policies BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(
        store
            .rebind_captured_with_policy(&f.scope, &export.id, 1, "New", &checkout, Some(&policy))
            .is_err()
    );
    assert_eq!(store.list(&f.scope).unwrap(), vec![export]);
    assert!(
        store
            .grants(&f.scope, &store.list(&f.scope).unwrap()[0].id)
            .unwrap()[0]
            .active
    );
    assert!(
        store
            .register_captured_with_policy(&f.scope, "Failed", &checkout, Some(&policy))
            .is_err()
    );
    assert_eq!(store.list(&f.scope).unwrap().len(), 1);
}
#[test]
fn persisted_corruption_denies_metadata_authorization_but_never_blocks_revocation() {
    let f = Fixture::new();
    let store = f.store();
    let policy = PolicySnapshot::parse(RAW.to_vec()).unwrap();
    let checkout = f.checkout(policy.digest());
    let export = store
        .register_captured_with_policy(&f.scope, "Policy", &checkout, Some(&policy))
        .unwrap();
    store
        .set_grant(
            &f.scope,
            &export.id,
            &principal(),
            &[ExportPermission::Read],
            true,
            None,
        )
        .unwrap();
    for sql in [
        "UPDATE export_execution_policies SET digest=printf('%064d',0)",
        "UPDATE export_execution_policies SET raw=x'7b7d'",
        "UPDATE export_execution_policies SET export_revision=2",
    ] {
        store
            .connection
            .execute(
                "UPDATE export_execution_policies SET raw=?1,digest=?2,export_revision=1",
                rusqlite::params![RAW, policy.digest()],
            )
            .unwrap();
        store.connection.execute_batch(sql).unwrap();
        assert!(
            store
                .candidate(&f.scope, &export.id, &principal(), ExportPermission::Read)
                .is_err()
        );
    }
    store.connection.execute_batch("INSERT INTO export_audit(export_id,principal_kind,principal_id,action,revision,at) SELECT 'fixture','export','','fixture',1,0 FROM (WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<4096) SELECT x FROM n);").unwrap();
    std::fs::remove_dir_all(f.dir.path().join("checkout")).unwrap();
    assert!(!store.revoke_export(&f.scope, &export.id, 1).unwrap().active);
}
#[test]
fn operator_migrates_v1_atomically_peer_open_never_creates_or_migrates() {
    let f = Fixture::new();
    let store = f.store();
    let policy = PolicySnapshot::parse(RAW.to_vec()).unwrap();
    let checkout = f.checkout(policy.digest());
    let export = store
        .register_captured(&f.scope, "Legacy", &checkout)
        .unwrap();
    store
        .set_grant(
            &f.scope,
            &export.id,
            &principal(),
            &[ExportPermission::Read],
            true,
            None,
        )
        .unwrap();
    store
        .connection
        .execute_batch("DROP TABLE worker_session_events;DROP TABLE worker_session_turns;DROP TABLE worker_sessions;DROP TABLE worker_session_meta;DROP TABLE worker_execution_outbox; DROP TABLE worker_execution_inbox; DROP TABLE worker_execution_meta; DROP TABLE export_execution_policies;PRAGMA user_version=1;")
        .unwrap();
    drop(store);
    let peer = WorkerStore::open_existing(&f.dir.path().join("state"), f.scope.clone()).unwrap();
    assert_eq!(
        peer.connection
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(
        peer.policy_snapshot(&export.id, checkout.settings_digest())
            .unwrap()
            .is_none()
    );
    assert!(
        peer.candidate(&f.scope, &export.id, &principal(), ExportPermission::Read)
            .unwrap()
            .validate_filesystem()
            .is_ok()
    );
    peer.connection
        .execute_batch("CREATE TABLE export_execution_policies(collision TEXT);")
        .unwrap();
    drop(peer);
    assert!(WorkerStore::open(&f.dir.path().join("state"), f.scope.clone()).is_err());
    let db = rusqlite::Connection::open(f.dir.path().join("state/exports.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.execute_batch("DROP TABLE export_execution_policies;")
        .unwrap();
    drop(db);
    let upgraded = f.store();
    assert!(upgraded.grants(&f.scope, &export.id).unwrap()[0].active);
    assert_eq!(upgraded.list(&f.scope).unwrap(), vec![export]);
    assert_eq!(
        upgraded
            .connection
            .query_row("SELECT count(*) FROM export_execution_policies", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    assert!(WorkerStore::open_existing(&f.dir.path().join("absent"), f.scope.clone()).is_err());
    assert!(!f.dir.path().join("absent").exists());
}
#[cfg(unix)]
#[test]
fn loader_rejects_fifo_symlinks_hardlinks_unsafe_modes_and_oversized_files() {
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{PermissionsExt, symlink},
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("policy.json");
    std::fs::write(&path, RAW).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let captured = PolicySnapshot::load(&path).unwrap();
    std::fs::write(&path, &RAW[..RAW.len() - 1]).unwrap();
    assert_eq!(captured.raw, RAW);
    assert_ne!(
        captured.digest(),
        PolicySnapshot::load(&path).unwrap().digest()
    );
    std::fs::write(&path, RAW).unwrap();
    let link = root.join("link");
    symlink(&path, &link).unwrap();
    assert!(PolicySnapshot::load(&link).is_err());
    let directory_link = root.join("directory-link");
    symlink(&root, &directory_link).unwrap();
    assert!(PolicySnapshot::load(&directory_link.join("policy.json")).is_err());
    let hard = root.join("hard");
    std::fs::hard_link(&path, &hard).unwrap();
    assert!(PolicySnapshot::load(&path).is_err());
    std::fs::remove_file(hard).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(PolicySnapshot::load(&path).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&path, vec![b' '; MAX_BYTES + 1]).unwrap();
    assert!(PolicySnapshot::load(&path).is_err());
    let fifo = root.join("fifo");
    let c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert!(PolicySnapshot::load(&fifo).is_err());
    assert!(PolicySnapshot::load(&root).is_err());
}

#[test]
fn linked_policy_requires_explicit_repository_wide_write_ack_and_fresh_execute_grant() {
    fn git(root: &Path, args: &[&str]) {
        let mut command = std::process::Command::new("git");
        dunlin_workspace::git::scrub_repository_environment(&mut command);
        assert!(
            command
                .current_dir(root)
                .args(args)
                .env("GIT_AUTHOR_NAME", "Fixture")
                .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
                .env("GIT_COMMITTER_NAME", "Fixture")
                .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let f = Fixture::new();
    let repository = f.dir.path().join("repository");
    let linked = f.dir.path().join("linked");
    std::fs::create_dir(&repository).unwrap();
    git(&repository, &["init", "-q"]);
    git(
        &repository,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-qm",
            "fixture",
        ],
    );
    git(
        &repository,
        &["worktree", "add", "-qb", "linked", linked.to_str().unwrap()],
    );
    let common = repository.join(".git").canonicalize().unwrap();
    let roots = crate::worker_exports::LinkedRoots {
        git_dir: common.join("worktrees/linked"),
        common_dir: common,
    };
    let policy = PolicySnapshot::parse(RAW.to_vec()).unwrap();
    let checkout = CapturedCheckout::capture(&linked, policy.digest(), Some(&roots)).unwrap();
    let store = f.store();
    assert!(
        store
            .register_captured_with_policy(&f.scope, "No acknowledgement", &checkout, Some(&policy))
            .is_err()
    );
    assert!(store.list(&f.scope).unwrap().is_empty());
    // Existing metadata-only permission cannot become process write authority.
    let export = store
        .register_captured(&f.scope, "Metadata", &checkout)
        .unwrap();
    store
        .set_grant(
            &f.scope,
            &export.id,
            &principal(),
            &[ExportPermission::Read, ExportPermission::Execute],
            true,
            None,
        )
        .unwrap();
    assert!(
        store
            .rebind_captured_with_policy(
                &f.scope,
                &export.id,
                1,
                "No acknowledgement",
                &checkout,
                Some(&policy)
            )
            .is_err()
    );
    assert!(store.grants(&f.scope, &export.id).unwrap()[0].active);
    let mut json: serde_json::Value = serde_json::from_slice(RAW).unwrap();
    json["acknowledge_repository_wide_metadata_write"] = serde_json::json!(true);
    let policy = PolicySnapshot::parse(serde_json::to_vec(&json).unwrap()).unwrap();
    let approved = CapturedCheckout::capture(&linked, policy.digest(), Some(&roots)).unwrap();
    let changed = store
        .rebind_captured_with_policy(
            &f.scope,
            &export.id,
            1,
            "Explicit approval",
            &approved,
            Some(&policy),
        )
        .unwrap();
    assert_eq!(changed.revision, 2);
    assert!(!store.grants(&f.scope, &export.id).unwrap()[0].active);
    assert!(
        store
            .candidate(
                &f.scope,
                &export.id,
                &principal(),
                ExportPermission::Execute
            )
            .is_err()
    );
    let revoked = store.grants(&f.scope, &export.id).unwrap().remove(0);
    store
        .set_grant(
            &f.scope,
            &export.id,
            &principal(),
            &[ExportPermission::Read, ExportPermission::Execute],
            true,
            Some(revoked.revision),
        )
        .unwrap();
    assert!(
        store
            .candidate(
                &f.scope,
                &export.id,
                &principal(),
                ExportPermission::Execute
            )
            .unwrap()
            .validate_filesystem()
            .is_ok()
    );
}

fn recipe_policy() -> serde_json::Value {
    let mut value: serde_json::Value = serde_json::from_slice(RAW).unwrap();
    value["schema_version"] = serde_json::json!(3);
    value["mcp_broker"] = serde_json::json!(true);
    value["recipes"] = serde_json::json!([{"program":"/opt/dunlin/deploy","template":{"name":"deploy","program_sha256":"a".repeat(64),"args":["{environment}"],"env_var":"API_TOKEN","parameters":[{"name":"environment","required":true,"max_len":32,"allow_leading_dash":false}],"timeout_secs":30}}]);
    value
}
#[test]
fn recipe_policy_requires_explicit_schema3_templates_and_cannot_upgrade_legacy_implicitly() {
    let original = recipe_policy();
    let raw = serde_json::to_vec(&original).unwrap();
    let parsed = PolicySnapshot::parse(raw.clone()).unwrap();
    assert!(parsed.permits_mcp_broker().unwrap());
    assert_eq!(parsed.raw, raw);
    for version in [1, 2] {
        let mut value = original.clone();
        value["schema_version"] = serde_json::json!(version);
        assert!(PolicySnapshot::parse(serde_json::to_vec(&value).unwrap()).is_err());
    }
    for value in [
        serde_json::Value::Null,
        serde_json::json!([]),
        serde_json::json!(false),
    ] {
        let mut policy = original.clone();
        policy["recipes"] = value;
        assert!(PolicySnapshot::parse(serde_json::to_vec(&policy).unwrap()).is_err());
    }
    for key in ["recipes", "mcp_broker"] {
        let mut value = original.clone();
        value.as_object_mut().unwrap().remove(key);
        assert!(PolicySnapshot::parse(serde_json::to_vec(&value).unwrap()).is_err());
    }
    let mut duplicate = original.clone();
    let recipe = duplicate["recipes"][0].clone();
    duplicate["recipes"].as_array_mut().unwrap().push(recipe);
    assert!(PolicySnapshot::parse(serde_json::to_vec(&duplicate).unwrap()).is_err());
    for (field, value) in [
        ("program_sha256", serde_json::json!("bad")),
        ("env_var", serde_json::json!("HOME")),
        ("timeout_secs", serde_json::json!(61)),
        ("secret", serde_json::json!("private")),
    ] {
        let mut policy = original.clone();
        policy["recipes"][0]["template"][field] = value;
        assert!(
            PolicySnapshot::parse(serde_json::to_vec(&policy).unwrap()).is_err(),
            "{field}"
        );
    }
}
#[test]
fn recipe_policy_change_rebinds_settings_and_invalidates_existing_export_authority() {
    let f = Fixture::new();
    let store = f.store();
    let mut value = recipe_policy();
    let policy = PolicySnapshot::parse(serde_json::to_vec(&value).unwrap()).unwrap();
    let checkout = f.checkout(policy.digest());
    let export = store
        .register_captured_with_policy(&f.scope, "Recipe", &checkout, Some(&policy))
        .unwrap();
    store
        .set_grant(
            &f.scope,
            &export.id,
            &principal(),
            &[ExportPermission::Read, ExportPermission::Execute],
            true,
            None,
        )
        .unwrap();
    let candidate = store
        .candidate(
            &f.scope,
            &export.id,
            &principal(),
            ExportPermission::Execute,
        )
        .unwrap()
        .validate_filesystem()
        .unwrap();
    let authority = store.authorize_validated(&f.scope, candidate).unwrap();
    value["recipes"][0]["template"]["program_sha256"] = serde_json::json!("b".repeat(64));
    let changed = PolicySnapshot::parse(serde_json::to_vec(&value).unwrap()).unwrap();
    assert_ne!(policy.digest(), changed.digest());
    assert!(
        store
            .rebind_captured_with_policy(
                &f.scope,
                &export.id,
                1,
                "Bad settings",
                &checkout,
                Some(&changed)
            )
            .is_err()
    );
    let recaptured = f.checkout(changed.digest());
    let rebound = store
        .rebind_captured_with_policy(
            &f.scope,
            &export.id,
            1,
            "New pin",
            &recaptured,
            Some(&changed),
        )
        .unwrap();
    assert_eq!(rebound.revision, 2);
    assert!(!store.grants(&f.scope, &export.id).unwrap()[0].active);
    assert!(store.revalidate_authority(&f.scope, &authority).is_err());
    let restored = store
        .policy_snapshot(&export.id, changed.digest())
        .unwrap()
        .unwrap();
    assert_eq!(restored.raw, changed.raw);
}
#[cfg(unix)]
#[test]
fn recipe_policy_cannot_select_a_program_inside_the_export_or_a_symlink_to_it() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let mut value = recipe_policy();
    let root = f.dir.path().join("checkout");
    std::fs::create_dir_all(&root).unwrap();
    let script = root.join("deploy");
    std::fs::write(&script, "fixture").unwrap();
    let link = f.dir.path().join("outside-link");
    symlink(&script, &link).unwrap();
    for path in [script, link] {
        value["recipes"][0]["program"] = serde_json::json!(path.to_str().unwrap());
        let policy = PolicySnapshot::parse(serde_json::to_vec(&value).unwrap()).unwrap();
        let checkout = f.checkout(policy.digest());
        assert!(
            f.store()
                .register_captured_with_policy(
                    &f.scope,
                    "Untrusted program",
                    &checkout,
                    Some(&policy)
                )
                .is_err()
        );
    }
}
