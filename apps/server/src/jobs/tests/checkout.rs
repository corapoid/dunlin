use super::*;
use crate::store::native_checkout::PreparedOrdinaryCheckout;
fn source(f: &Fixture) -> NativeCheckoutAuthority {
    let candidate = f
        .state
        .store
        .lock()
        .unwrap()
        .native_checkout_candidate(
            &f.task.request.project_id,
            &f.user,
            MetadataPermission::Read,
        )
        .unwrap();
    let validated = candidate.validate_filesystem().unwrap();
    f.state
        .store
        .lock()
        .unwrap()
        .authorize_native_checkout(validated)
        .unwrap()
}
#[tokio::test]
async fn missing_source_stamp_survives_reopen_and_never_spawns_or_creates_checkout() {
    let mut f = Fixture::new("ok");
    f.task.source_checkout_stamp = None;
    let record = f.claim();
    let reopened = Store::open(&f.state.config.db).unwrap();
    let persisted = reopened.job(&record.job.id).unwrap().unwrap();
    assert!(persisted.task.source_checkout_stamp.is_none());
    assert!(persisted.checkout_stamp.is_none());
    let error = runner::execute(&f.state, &persisted).await.unwrap_err();
    assert_eq!(error.0, Status::Blocked);
    assert!(error.1.contains("Legacy task"));
    assert!(!f.checkout(&record.job.id).exists());
}
#[test]
fn derived_project_grant_and_stamp_publication_roll_back_together_and_reopen_exactly() {
    let f = Fixture::new("ok");
    let record = f.claim();
    let authority = source(&f);
    let root = f.checkout(&record.job.id);
    fs::create_dir_all(&root).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    let prepared =
        PreparedOrdinaryCheckout::capture(&root, &crate::auth::hash_token("{}")).unwrap();
    let peer = rusqlite::Connection::open(&f.state.config.db).unwrap();
    let count: i64 = peer
        .query_row("SELECT count(*) FROM projects", [], |r| r.get(0))
        .unwrap();
    peer.execute_batch("CREATE TRIGGER reject_checkout_stamp BEFORE UPDATE OF checkout_stamp ON relay_jobs BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    let publish = |prepared| {
        f.state.store.lock().unwrap().publish_job_checkout(
            &record.job.id,
            &authority,
            prepared,
            "fixture-branch",
            |store, current| {
                authorized_in_store(&f.state, store, current)
                    .map_err(crate::store::StoreError::Conflict)
            },
        )
    };
    assert!(publish(prepared.clone()).is_err());
    assert_eq!(
        count,
        peer.query_row("SELECT count(*) FROM projects", [], |r| r.get::<_, i64>(0))
            .unwrap()
    );
    let old = f
        .state
        .store
        .lock()
        .unwrap()
        .job(&record.job.id)
        .unwrap()
        .unwrap();
    assert!(old.checkout_stamp.is_none());
    assert!(old.job.checkout_project.is_none());
    assert_eq!(old.job.status, Status::Preparing);
    peer.execute_batch("DROP TRIGGER reject_checkout_stamp")
        .unwrap();
    let running = publish(prepared).unwrap();
    let pin = running.checkout_stamp.clone().unwrap();
    assert_eq!(pin.permission, MetadataPermission::Write);
    assert!(pin.metadata.is_none());
    assert_ne!(pin.native_grant.project_id, f.task.request.project_id);
    assert_eq!(
        serde_json::to_value(&running.task).unwrap(),
        serde_json::to_value(&record.task).unwrap()
    );
    let reopened = Store::open(&f.state.config.db).unwrap();
    assert_eq!(
        reopened
            .job(&record.job.id)
            .unwrap()
            .unwrap()
            .checkout_stamp,
        Some(pin)
    );
}
#[test]
fn stamp_dto_roundtrip_never_constructs_authority_or_tolerates_unknown_fields() {
    let f = Fixture::new("ok");
    let stamp = source(&f).stamp();
    let mut value = serde_json::to_value(&stamp).unwrap();
    assert_eq!(
        serde_json::from_value::<NativeCheckoutStamp>(value.clone()).unwrap(),
        stamp
    );
    value["root"] = serde_json::json!("/unapproved");
    assert!(serde_json::from_value::<NativeCheckoutStamp>(value).is_err());
}
fn link_source(f: &mut Fixture) -> std::path::PathBuf {
    let linked = f.dir.path().join("linked-source");
    let output = Command::new("git")
        .args(["-C", &f.task.root, "worktree", "add", "--detach"])
        .arg(&linked)
        .arg(&f.task.head)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let git = String::from_utf8(
        Command::new("git")
            .arg("-C")
            .arg(&linked)
            .args(["rev-parse", "--absolute-git-dir"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let common = std::path::Path::new(&f.task.root).join(".git");
    let store = f.state.store.lock().unwrap();
    let project = store
        .create_project("linked-source", linked.to_str().unwrap())
        .unwrap();
    store.grant(&project.id, &f.user).unwrap();
    let prepared = store
        .prepare_local_metadata(&project.id)
        .unwrap()
        .capture(std::path::Path::new(git.trim()), &common)
        .unwrap();
    store.commit_local_metadata(prepared, None).unwrap();
    assert!(
        store
            .native_checkout_candidate(&project.id, &f.user, MetadataPermission::Read)
            .is_err()
    );
    store
        .set_local_metadata_grant(
            &project.id,
            &f.user,
            1,
            1,
            &[MetadataPermission::Read],
            true,
            None,
            false,
        )
        .unwrap();
    f.task.request.project_id = project.id.clone();
    f.task.native_grant = Some(store.capture_native_grant(&project.id, &f.user).unwrap());
    f.task.root = linked.to_str().unwrap().into();
    drop(store);
    let authority = source(f);
    f.task.source_checkout_stamp = Some(authority.stamp());
    common
}
#[tokio::test]
async fn linked_source_read_consent_clones_into_independent_ordinary_write_checkout() {
    let mut f = Fixture::new("ok");
    link_source(&mut f);
    let record = f.claim();
    let report = runner::execute(&f.state, &record).await.unwrap();
    assert!(report.contains("fixture review"));
    let saved = Store::open(&f.state.config.db)
        .unwrap()
        .job(&record.job.id)
        .unwrap()
        .unwrap();
    let source = saved.task.source_checkout_stamp.unwrap();
    let derived = saved.checkout_stamp.unwrap();
    assert_eq!(source.permission, MetadataPermission::Read);
    assert!(source.metadata.is_some());
    assert_eq!(derived.permission, MetadataPermission::Write);
    assert!(derived.metadata.is_none());
    assert_ne!(
        derived.native_grant.project_id,
        source.native_grant.project_id
    );
}
#[tokio::test]
async fn linked_source_and_derived_object_swap_stop_real_paid_turn_before_late_mutation() {
    for target in [
        "source-grant",
        "source-aba",
        "source-objects",
        "source-alias",
        "derived-objects",
    ] {
        let mut f = Fixture::new("hang");
        let common = link_source(&mut f);
        let original = include_str!("fake_acp.py");
        let delayed=original.replace("while True:\n                time.sleep(1)","time.sleep(2)\n            open('late-object-mutation','w').write('must not execute')\n            while True:\n                time.sleep(1)");
        assert_ne!(delayed, original);
        fs::write(f.dir.path().join("fake.py"), delayed).unwrap();
        let record = f.claim();
        let marker = f.checkout(&record.job.id).join("started");
        let checkout = f.checkout(&record.job.id);
        let state = f.state.clone();
        let project = f.task.request.project_id.clone();
        let owner = f.user.clone();
        let source_root = std::path::PathBuf::from(&f.task.root);
        let control = async move {
            let pid = tokio::time::timeout(Duration::from_secs(8), async {
                loop {
                    if let Ok(raw) = fs::read_to_string(&marker)
                        && let Ok(pid) = raw.parse::<i32>()
                    {
                        break pid;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
            match target {
                "source-grant" => {
                    state
                        .store
                        .lock()
                        .unwrap()
                        .revoke_local_metadata_grant(&project, &owner, 1)
                        .unwrap();
                }
                "source-aba" => {
                    let store = state.store.lock().unwrap();
                    store
                        .revoke_local_metadata_grant(&project, &owner, 1)
                        .unwrap();
                    store
                        .set_local_metadata_grant(
                            &project,
                            &owner,
                            1,
                            1,
                            &[MetadataPermission::Read],
                            true,
                            Some(2),
                            false,
                        )
                        .unwrap();
                }
                "source-alias" => {
                    #[cfg(unix)]
                    {
                        let old = source_root.with_file_name("previous-linked-source");
                        fs::rename(&source_root, &old).unwrap();
                        let other = source_root.with_file_name("unapproved-source");
                        fs::create_dir(&other).unwrap();
                        std::os::unix::fs::symlink(&other, &source_root).unwrap();
                    }
                }
                "source-objects" => {
                    fs::rename(common.join("objects"), common.join("old-objects")).unwrap();
                    fs::create_dir(common.join("objects")).unwrap();
                }
                _ => {
                    fs::rename(
                        checkout.join(".git/objects"),
                        checkout.join(".git/old-objects"),
                    )
                    .unwrap();
                    fs::create_dir(checkout.join(".git/objects")).unwrap();
                }
            }
            pid
        };
        let (result, pid) = tokio::join!(
            runner::execute_until(
                &f.state,
                &record,
                tokio::time::Instant::now() + Duration::from_secs(12)
            ),
            control
        );
        assert_eq!(result.unwrap_err().0, Status::Blocked);
        assert!(
            !f.checkout(&record.job.id)
                .join("late-object-mutation")
                .exists()
        );
        #[cfg(unix)]
        assert_ne!(
            unsafe { libc::kill(pid, 0) },
            0,
            "Revoked real agent process survived"
        );
    }
}
#[cfg(unix)]
#[tokio::test]
async fn changing_sql_path_to_another_alias_of_same_root_never_restores_old_approval() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new("ok");
    let record = f.claim();
    let alias = f.dir.path().join("alias");
    symlink(&f.task.root, &alias).unwrap();
    let peer = rusqlite::Connection::open(&f.state.config.db).unwrap();
    peer.execute(
        "UPDATE projects SET path=?2 WHERE id=?1",
        rusqlite::params![f.task.request.project_id, alias.to_str().unwrap()],
    )
    .unwrap();
    assert!(authorized(&f.state, &record).is_err());
    let error = runner::execute(&f.state, &record).await.unwrap_err();
    assert_eq!(error.0, Status::Blocked);
    assert!(!f.checkout(&record.job.id).exists());
}
#[test]
fn independent_publishers_race_one_reservation_without_duplicate_native_grants() {
    let f = Fixture::new("ok");
    let record = f.claim();
    let authority = source(&f);
    let root = f.checkout(&record.job.id);
    fs::create_dir_all(&root).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .arg(&root)
            .status()
            .unwrap()
            .success()
    );
    let prepared =
        PreparedOrdinaryCheckout::capture(&root, &crate::auth::hash_token("{}")).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let path = f.state.config.db.clone();
        let state = f.state.clone();
        let id = record.job.id.clone();
        let authority = authority.clone();
        let prepared = prepared.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            let store = Store::open(path).unwrap();
            barrier.wait();
            store.publish_job_checkout(
                &id,
                &authority,
                prepared,
                "race-branch",
                |store, current| {
                    authorized_in_store(&state, store, current)
                        .map_err(crate::store::StoreError::Conflict)
                },
            )
        }));
    }
    let results = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(
        results
            .iter()
            .any(|r| matches!(r, Err(crate::store::StoreError::Conflict(_))))
    );
    let store = f.state.store.lock().unwrap();
    assert_eq!(store.list_projects().unwrap().len(), 2);
    let saved = store.job(&record.job.id).unwrap().unwrap();
    assert_eq!(saved.job.status, Status::Running);
    assert!(saved.checkout_stamp.is_some());
}
