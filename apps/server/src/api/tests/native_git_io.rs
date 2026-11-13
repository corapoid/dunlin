use super::*;
use crate::api::native_io::{NativeGitIo, NativeIo};
use crate::store::local_metadata::MetadataPermission;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

fn identity(f: &Fixture) -> (String, String) {
    let (u, d) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    (u.id, d.id)
}
async fn capture(f: &Fixture, permission: MetadataPermission) -> NativeGitIo {
    let (u, d) = identity(f);
    NativeGitIo::capture(&f.state, &u, &d, &f.project, permission)
        .await
        .unwrap()
}
fn aba(f: &Fixture) {
    let (u, _) = identity(f);
    let store = f.state.store.lock().unwrap();
    store.revoke_grant(&f.project, &u).unwrap();
    store.grant(&f.project, &u).unwrap();
}
#[tokio::test]
async fn git_read_withholds_real_bytes_after_native_grant_aba() {
    let f = Fixture::new();
    fs::write(f.root.join("secret"), "GIT_PRIVATE_OUTPUT_MARKER").unwrap();
    let io = capture(&f, MetadataPermission::Read).await;
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let task = tokio::spawn(io.read(move |project| {
        let bytes = fs::read(std::path::Path::new(&project.path).join("secret"))
            .map_err(|e| e.to_string())?;
        entered.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(3))
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    }));
    tokio::time::timeout(Duration::from_secs(3), started)
        .await
        .unwrap()
        .unwrap();
    aba(&f);
    release.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert!(!error.message.contains("GIT_PRIVATE_OUTPUT_MARKER"));
}
#[tokio::test]
async fn git_prepared_write_pins_settings_and_rejects_before_side_effect() {
    let f = Fixture::new();
    let io = capture(&f, MetadataPermission::Write).await;
    let proof = io.proof();
    f.state
        .store
        .lock()
        .unwrap()
        .save_project_settings(&f.project, "{\"changed\":true}")
        .unwrap();
    assert!(NativeGitIo::resume(&f.state, proof).await.is_err());
    let called = Arc::new(AtomicBool::new(false));
    let c = called.clone();
    assert!(
        io.mutate(move |_| {
            c.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await
        .is_err()
    );
    assert!(!called.load(Ordering::SeqCst));
}
#[tokio::test]
async fn read_checkout_proof_cannot_authorize_mutation_and_init_has_narrow_terminal_shape() {
    let f = Fixture::new();
    let read = capture(&f, MetadataPermission::Read).await;
    let called = Arc::new(AtomicBool::new(false));
    let c = called.clone();
    let error = read
        .mutate(move |_| {
            c.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert!(!called.load(Ordering::SeqCst));
    let write = capture(&f, MetadataPermission::Write).await;
    let stale = write.proof();
    write
        .initialize(|project| dunlin_workspace::git::initialize(&project.path))
        .await
        .unwrap();
    assert!(f.root.join(".git").is_dir());
    assert!(
        NativeGitIo::resume(&f.state, stale).await.is_err(),
        "Init must not refresh a reusable Directory proof"
    );
    // Reinitializing an existing ordinary repository keeps its identity; it
    // must use the generic postcheck, not a Directory-only transition.
    let existing = capture(&f, MetadataPermission::Write).await;
    let same_repository = existing.proof();
    existing
        .initialize(|project| dunlin_workspace::git::initialize(&project.path))
        .await
        .unwrap();
    NativeGitIo::resume(&f.state, same_repository)
        .await
        .unwrap();
    capture(&f, MetadataPermission::Read)
        .await
        .validate()
        .await
        .unwrap();
}

fn linked(f: &Fixture) -> (String, String, std::path::PathBuf) {
    let common = f
        .root
        .parent()
        .unwrap()
        .join(format!("metadata-{}", uuid::Uuid::now_v7()));
    let git = common.join("worktrees/test");
    fs::create_dir_all(&git).unwrap();
    fs::create_dir_all(common.join("objects")).unwrap();
    fs::write(f.root.join(".git"), format!("gitdir: {}\n", git.display())).unwrap();
    fs::write(git.join("commondir"), "../..").unwrap();
    fs::write(git.join("gitdir"), f.root.join(".git").to_str().unwrap()).unwrap();
    fs::write(git.join("HEAD"), "ref: refs/heads/main").unwrap();
    fs::write(common.join("HEAD"), "ref: refs/heads/main").unwrap();
    let (u, d) = identity(f);
    let store = f.state.store.lock().unwrap();
    let approved = store
        .prepare_local_metadata(&f.project)
        .unwrap()
        .capture(&git, &common)
        .unwrap();
    store.commit_local_metadata(approved, None).unwrap();
    store
        .set_local_metadata_grant(
            &f.project,
            &u,
            1,
            1,
            &[MetadataPermission::Read],
            true,
            None,
            false,
        )
        .unwrap();
    (u, d, common)
}
#[tokio::test]
async fn linked_git_requires_own_metadata_grant_without_expanding_plain_workspace_access() {
    let f = Fixture::new();
    let (u, d, common) = linked(&f);
    let read = capture(&f, MetadataPermission::Read).await;
    assert!(read.authority().metadata_stamp().is_some());
    assert!(
        NativeGitIo::capture(&f.state, &u, &d, &f.project, MetadataPermission::Write)
            .await
            .is_err()
    );
    let (other, device) = {
        let s = f.state.store.lock().unwrap();
        let u = s.create_user("no-metadata", "member").unwrap();
        s.grant(&f.project, &u.id).unwrap();
        let (d, _) = s.create_device(&u.id, "test").unwrap();
        (u, d)
    };
    assert!(
        NativeGitIo::capture(
            &f.state,
            &other.id,
            &device.id,
            &f.project,
            MetadataPermission::Read
        )
        .await
        .is_err()
    );
    let plain = NativeIo::capture(&f.state, &u, &d, &f.project).unwrap();
    assert_eq!(plain.project.path, f.root.to_str().unwrap());
    assert!(
        plain
            .read(|p| dunlin_workspace::fs::read_file(&p.path, "../outside"))
            .await
            .is_err()
    );
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_local_metadata_grant(&f.project, &u, 1)
        .unwrap();
    assert!(read.validate().await.is_err());
    assert!(
        NativeIo::capture(&f.state, &u, &d, &f.project).is_ok(),
        "Workspace files remain independent of metadata approval"
    );
    fs::remove_dir_all(common).unwrap();
}

#[tokio::test]
async fn actual_git_http_rejects_unapproved_linked_shape_without_changing_workspace_file_policy() {
    let f = Fixture::new();
    let (_, _, common) = linked(&f);
    let (u, _) = identity(&f);
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_local_metadata_grant(&f.project, &u, 1)
        .unwrap();
    let response = router(f.state.clone())
        .oneshot(
            Request::get(format!("/v1/git/status?project={}", f.project))
                .header("Authorization", format!("Bearer {}", f.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = f
        .post(
            "/v1/fs/upload",
            Some(&f.token),
            json!({"project":f.project,"name":"safe.txt","contents":"c2FmZQ=="}),
        )
        .await;
    assert_eq!(
        response.0,
        StatusCode::OK,
        "Workspace upload must not borrow or require the metadata grant: {:?}",
        response.1
    );
    assert!(
        !common.join("info/exclude").exists(),
        "An unapproved metadata write must never accompany workspace upload"
    );
    fs::remove_dir_all(common).unwrap();
}

#[tokio::test]
async fn linked_upload_appends_only_through_own_write_metadata_authority() {
    let f = Fixture::new();
    let (u, _, common) = linked(&f);
    let read = capture(&f, MetadataPermission::Read).await;
    assert!(read.authority().open_git_exclude().is_err());
    f.state
        .store
        .lock()
        .unwrap()
        .set_local_metadata_grant(
            &f.project,
            &u,
            1,
            1,
            &[MetadataPermission::Read, MetadataPermission::Write],
            true,
            Some(1),
            true,
        )
        .unwrap();
    fs::create_dir(common.join("info")).unwrap();
    fs::write(common.join("info/exclude"), "# operator-owned\nkeep-me\n").unwrap();
    let response = f
        .post(
            "/v1/fs/upload",
            Some(&f.token),
            json!({"project":f.project,"name":"safe.txt","contents":"c2FmZQ=="}),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK, "{:?}", response.1);
    assert_eq!(
        fs::read_to_string(common.join("info/exclude")).unwrap(),
        "# operator-owned\nkeep-me\n.dunlin/\n"
    );
    let write = capture(&f, MetadataPermission::Write).await;
    aba(&f);
    assert!(crate::api::exclude_attachments_from_git(&write).is_err());
    assert_eq!(
        fs::read_to_string(common.join("info/exclude")).unwrap(),
        "# operator-owned\nkeep-me\n.dunlin/\n"
    );
    fs::remove_dir_all(common).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn metadata_exclude_rejects_info_symlink_exclude_symlink_and_hardlink() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let (u, _, common) = linked(&f);
    f.state
        .store
        .lock()
        .unwrap()
        .set_local_metadata_grant(
            &f.project,
            &u,
            1,
            1,
            &[MetadataPermission::Read, MetadataPermission::Write],
            true,
            Some(1),
            true,
        )
        .unwrap();
    let write = capture(&f, MetadataPermission::Write).await;
    let outside = f.root.join("outside-info");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("exclude"), "DO_NOT_TOUCH\n").unwrap();
    symlink(&outside, common.join("info")).unwrap();
    assert!(crate::api::exclude_attachments_from_git(&write).is_err());
    fs::remove_file(common.join("info")).unwrap();
    fs::create_dir(common.join("info")).unwrap();
    symlink(outside.join("exclude"), common.join("info/exclude")).unwrap();
    assert!(crate::api::exclude_attachments_from_git(&write).is_err());
    fs::remove_file(common.join("info/exclude")).unwrap();
    fs::hard_link(outside.join("exclude"), common.join("info/exclude")).unwrap();
    assert!(crate::api::exclude_attachments_from_git(&write).is_err());
    assert_eq!(
        fs::read_to_string(outside.join("exclude")).unwrap(),
        "DO_NOT_TOUCH\n"
    );
    fs::remove_dir_all(common).unwrap();
}

#[tokio::test]
async fn metadata_revocation_after_actual_write_returns_unknown_without_replay() {
    let f = Fixture::new();
    let (u, _, common) = linked(&f);
    f.state
        .store
        .lock()
        .unwrap()
        .set_local_metadata_grant(
            &f.project,
            &u,
            1,
            1,
            &[MetadataPermission::Read, MetadataPermission::Write],
            true,
            Some(1),
            true,
        )
        .unwrap();
    let write = capture(&f, MetadataPermission::Write).await;
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = count.clone();
    let task = tokio::spawn(write.mutate(move |project| {
        c.fetch_add(1, Ordering::SeqCst);
        fs::write(
            std::path::Path::new(&project.path).join("written-before-revoke"),
            "effect",
        )
        .map_err(|e| e.to_string())?;
        entered.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(3))
            .map_err(|e| e.to_string())?;
        Ok(())
    }));
    tokio::time::timeout(Duration::from_secs(3), started)
        .await
        .unwrap()
        .unwrap();
    f.state
        .store
        .lock()
        .unwrap()
        .revoke_local_metadata_grant(&f.project, &u, 2)
        .unwrap();
    release.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.status, StatusCode::CONFLICT);
    assert!(error.message.contains("do not automatically retry"));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(f.root.join("written-before-revoke").exists());
    fs::remove_dir_all(common).unwrap();
}

#[tokio::test]
async fn public_project_copy_cannot_redirect_a_captured_git_operation() {
    let f = Fixture::new();
    let mut io = capture(&f, MetadataPermission::Write).await;
    io.project.path = f.root.parent().unwrap().to_string_lossy().into_owned();
    let called = Arc::new(AtomicBool::new(false));
    let observed = called.clone();
    let error = io
        .mutate(move |_| {
            observed.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert!(!called.load(Ordering::SeqCst));
}
