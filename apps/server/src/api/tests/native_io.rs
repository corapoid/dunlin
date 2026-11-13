use super::*;
use crate::api::native_io::NativeIo;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

fn scope(f: &Fixture) -> NativeIo {
    let (user, device) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    NativeIo::capture(&f.state, &user.id, &device.id, &f.project).unwrap()
}
fn revoke_and_regrant(f: &Fixture) {
    let store = f.state.store.lock().unwrap();
    let user = store.authenticate(&f.token).unwrap().unwrap().0;
    store.revoke_grant(&f.project, &user.id).unwrap();
    store.grant(&f.project, &user.id).unwrap();
    assert!(
        store
            .project_for_user(&f.project, &user.id)
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn native_io_read_withholds_actual_file_bytes_after_atomic_revoke_regrant() {
    let f = Fixture::new();
    fs::write(f.root.join("private.txt"), "NATIVE_IO_PRIVATE_FILE_MARKER").unwrap();
    let guard = scope(&f);
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let task = tokio::spawn(guard.read(move |project| {
        let bytes = fs::read(std::path::Path::new(&project.path).join("private.txt"))
            .map_err(|error| error.to_string())?;
        entered.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(3))
            .map_err(|error| error.to_string())?;
        Ok(bytes)
    }));
    tokio::time::timeout(Duration::from_secs(3), started)
        .await
        .unwrap()
        .unwrap();
    revoke_and_regrant(&f);
    release.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert!(!error.message.contains("NATIVE_IO_PRIVATE_FILE_MARKER"));
    assert!(
        scope(&f)
            .read(
                |project| fs::read(std::path::Path::new(&project.path).join("private.txt"))
                    .map_err(|error| error.to_string())
            )
            .await
            .is_ok(),
        "A fresh explicit request may use the new epoch"
    );
}

#[tokio::test]
async fn native_io_mutation_captured_before_regrant_never_invokes_its_write() {
    let f = Fixture::new();
    let guard = scope(&f);
    let called = Arc::new(AtomicBool::new(false));
    let worker_called = called.clone();
    revoke_and_regrant(&f);
    let result = guard
        .mutate(move |project| {
            worker_called.store(true, Ordering::SeqCst);
            fs::write(
                std::path::Path::new(&project.path).join("forbidden-write"),
                "must not appear",
            )
            .map_err(|error| error.to_string())
        })
        .await;
    assert_eq!(result.unwrap_err().status, StatusCode::FORBIDDEN);
    assert!(!called.load(Ordering::SeqCst));
    assert!(!f.root.join("forbidden-write").exists());
}

#[tokio::test]
async fn native_io_mutation_losing_epoch_after_write_reports_uncertain_result_without_replay() {
    let f = Fixture::new();
    let guard = scope(&f);
    let count = Arc::new(AtomicUsize::new(0));
    let worker_count = count.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let task = tokio::spawn(guard.mutate(move |project| {
        worker_count.fetch_add(1, Ordering::SeqCst);
        fs::write(
            std::path::Path::new(&project.path).join("written-before-revoke"),
            "actual side effect",
        )
        .map_err(|error| error.to_string())?;
        entered.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(3))
            .map_err(|error| error.to_string())?;
        Ok(())
    }));
    tokio::time::timeout(Duration::from_secs(3), started)
        .await
        .unwrap()
        .unwrap();
    revoke_and_regrant(&f);
    release.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.status, StatusCode::CONFLICT);
    assert!(error.message.contains("partial"));
    assert!(error.message.contains("do not automatically retry"));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        fs::read_to_string(f.root.join("written-before-revoke")).unwrap(),
        "actual side effect"
    );
}

#[test]
fn actual_http_file_read_queued_before_regrant_returns_no_bytes_and_releases_its_permit() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let f = Fixture::new();
        fs::write(f.root.join("private.txt"), "HTTP_NATIVE_IO_PRIVATE_MARKER").unwrap();
        let permits = f.state.fs_reads.available_permits();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
        });
        started.await.unwrap();
        let request = Request::get(format!(
            "/v1/fs/file?project={}&path=private.txt",
            f.project
        ))
        .header("authorization", format!("Bearer {}", f.token))
        .body(Body::empty())
        .unwrap();
        let state = f.state.clone();
        let response = tokio::spawn(async move { router(state).oneshot(request).await.unwrap() });
        // The permit is acquired after NativeIo capture and moved into the queued
        // blocking closure. This is a deterministic HTTP entry/execution barrier.
        tokio::time::timeout(Duration::from_secs(3), async {
            while f.state.fs_reads.available_permits() == permits {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        revoke_and_regrant(&f);
        release.send(()).unwrap();
        blocker.await.unwrap();
        let response = response.await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("HTTP_NATIVE_IO_PRIVATE_MARKER"));
        assert_eq!(f.state.fs_reads.available_permits(), permits);
    });
}

async fn upload_artifact(f: &Fixture) -> Value {
    use base64::Engine;
    let (status,row)=f.post("/v1/artifacts",Some(&f.token),json!({"project_id":f.project,"kind":"fixture","title":"Private fixture","mime":"text/plain","data":base64::engine::general_purpose::STANDARD.encode("NATIVE_ARTIFACT_PRIVATE_MARKER")})).await;
    assert_eq!(status, StatusCode::OK, "{row}");
    row
}

#[tokio::test]
async fn artifact_public_capability_survives_native_revoke_but_project_reader_cannot_mutate_owner_metadata()
 {
    let f = Fixture::new();
    let row = upload_artifact(&f).await;
    let id = row["id"].as_str().unwrap();
    {
        let store = f.state.store.lock().unwrap();
        let peer = store.authenticate(&f.ungranted_token).unwrap().unwrap().0;
        store.grant(&f.project, &peer.id).unwrap();
    }
    assert_eq!(
        f.get_json(&format!("/v1/artifacts/{id}"), Some(&f.ungranted_token))
            .await
            .0,
        StatusCode::OK,
        "Existing project-granted private read semantics remain available"
    );
    let (_, peer_list) = f
        .get_json(
            &format!("/v1/artifacts?project={}", f.project),
            Some(&f.ungranted_token),
        )
        .await;
    assert_eq!(peer_list, json!([]), "Artifact lists remain owner-specific");
    assert_eq!(
        f.delete(&format!("/v1/artifacts/{id}"), Some(&f.ungranted_token))
            .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.post(
            &format!("/v1/artifacts/{id}/share"),
            Some(&f.ungranted_token),
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.delete(
            &format!("/v1/artifacts/{id}/share"),
            Some(&f.ungranted_token)
        )
        .await,
        StatusCode::FORBIDDEN
    );
    let (status, share) = f
        .post(
            &format!("/v1/artifacts/{id}/share"),
            Some(&f.token),
            json!({"ttl_secs":3600}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let url = share["url"].as_str().unwrap();
    let owner = {
        let store = f.state.store.lock().unwrap();
        let owner = store.authenticate(&f.token).unwrap().unwrap().0;
        store.revoke_grant(&f.project, &owner.id).unwrap();
        owner.id
    };
    assert_eq!(
        f.get_json(&format!("/v1/artifacts/{id}"), Some(&f.token))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, bytes) = f.get_raw(url).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, b"NATIVE_ARTIFACT_PRIVATE_MARKER");
    f.state
        .store
        .lock()
        .unwrap()
        .grant(&f.project, &owner)
        .unwrap();
    assert_eq!(
        f.delete(&format!("/v1/artifacts/{id}/share"), Some(&f.token))
            .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(f.get_raw(url).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        f.delete(&format!("/v1/artifacts/{id}"), Some(&f.token))
            .await,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn artifact_staging_cannot_publish_or_register_after_source_grant_epoch_changes() {
    let f = Fixture::new();
    let io = scope(&f);
    let directory = f.state.artifact_dir.join(&f.project);
    fs::create_dir_all(&directory).unwrap();
    let id = uuid::Uuid::now_v7().to_string();
    let pending = directory.join(format!(".pending-{id}"));
    let destination = directory.join(&id);
    fs::write(&pending, "PRIVATE_STAGED_ARTIFACT").unwrap();
    let mut stage = crate::api::ArtifactStage {
        path: pending.clone(),
        published: false,
    };
    let row = crate::store::ArtifactRow {
        id: id.clone(),
        project_id: f.project.clone(),
        user_id: io.proof().user,
        kind: "fixture".into(),
        title: "staged".into(),
        mime: "text/plain".into(),
        bytes: 23,
        sha256: "fixture".into(),
        blob_path: destination.to_string_lossy().into_owned(),
        created_at: crate::store::now_secs(),
    };
    revoke_and_regrant(&f);
    assert_eq!(
        crate::api::publish_artifact(&f.state, &io, &row, &mut stage)
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    assert!(!destination.exists());
    assert!(
        f.state
            .store
            .lock()
            .unwrap()
            .artifact(&id)
            .unwrap()
            .is_none()
    );
    drop(stage);
    assert!(
        !pending.exists(),
        "Unpublished server-owned bytes must be cleaned"
    );
}

#[tokio::test]
async fn stale_artifact_metadata_proof_cannot_delete_or_revoke_an_existing_capability() {
    let f = Fixture::new();
    let row = upload_artifact(&f).await;
    let id = row["id"].as_str().unwrap();
    let (status, share) = f
        .post(
            &format!("/v1/artifacts/{id}/share"),
            Some(&f.token),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (user, device) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    let (original, io) = crate::api::private_artifact(&f.state, &user.id, &device.id, id).unwrap();
    revoke_and_regrant(&f);
    let revoked = crate::api::mutate_owned_artifact(&f.state, &io, &original, |store, row| {
        store.revoke_artifact_shares(&row.id)?;
        Ok(())
    });
    assert_eq!(revoked.unwrap_err().status, StatusCode::FORBIDDEN);
    let removed = crate::api::mutate_owned_artifact(&f.state, &io, &original, |store, row| {
        store.delete_artifact(&row.id)?;
        Ok(())
    });
    assert_eq!(removed.unwrap_err().status, StatusCode::FORBIDDEN);
    assert!(
        f.state
            .store
            .lock()
            .unwrap()
            .artifact(id)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        f.get_raw(share["url"].as_str().unwrap()).await.0,
        StatusCode::OK
    );
}

#[test]
fn actual_private_artifact_http_read_queued_under_an_old_epoch_returns_no_payload() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let f = Fixture::new();
        let row = upload_artifact(&f).await;
        let id = row["id"].as_str().unwrap();
        let permits = f.state.fs_reads.available_permits();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
        });
        started.await.unwrap();
        let request = Request::get(format!("/v1/artifacts/{id}"))
            .header("authorization", format!("Bearer {}", f.token))
            .body(Body::empty())
            .unwrap();
        let state = f.state.clone();
        let response = tokio::spawn(async move { router(state).oneshot(request).await.unwrap() });
        tokio::time::timeout(Duration::from_secs(3), async {
            while f.state.fs_reads.available_permits() == permits {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        revoke_and_regrant(&f);
        release.send(()).unwrap();
        blocker.await.unwrap();
        let response = response.await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = String::from_utf8(to_bytes(response.into_body(), 4096).await.unwrap().to_vec())
            .unwrap();
        assert!(!body.contains("NATIVE_ARTIFACT_PRIVATE_MARKER"));
        use base64::Engine;
        assert!(!body.contains(
            &base64::engine::general_purpose::STANDARD.encode("NATIVE_ARTIFACT_PRIVATE_MARKER")
        ));
        assert_eq!(f.state.fs_reads.available_permits(), permits);
    });
}

#[tokio::test]
async fn github_metadata_never_trusts_an_extracted_admin_snapshot_over_current_member_role() {
    let f = Fixture::new();
    let (mut user, device) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    user.role = "admin".into(); // Deliberately stale/forged extractor snapshot in a unit call.
    let result = crate::api::github_metadata(
        crate::api::AuthUser { user, device },
        axum::extract::State(f.state.clone()),
        axum::Json(crate::api::GithubLoadRequest {
            project: f.project.clone(),
        }),
    )
    .await;
    assert_eq!(
        result.unwrap_err().status,
        StatusCode::FORBIDDEN,
        "No CLI/provider call may occur before a fresh admin check"
    );
}

#[tokio::test]
async fn current_admin_postflight_check_withholds_local_metadata_after_role_downgrade() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let store = crate::store::Store::open(&path).unwrap();
    let user = store.create_user("metadata-admin", "admin").unwrap();
    let project = store
        .create_project("metadata", directory.path().to_str().unwrap())
        .unwrap();
    store.grant(&project.id, &user.id).unwrap();
    let (device, _) = store.create_device(&user.id, "metadata").unwrap();
    let state = crate::api::AppState::new(
        store,
        crate::config::Config {
            db: path.clone(),
            sandbox: false,
            ..Default::default()
        },
    );
    let io = NativeIo::capture(&state, &user.id, &device.id, &project.id).unwrap();
    let worker_state = state.clone();
    let worker_user = user.id.clone();
    let worker_device = device.id.clone();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let task = tokio::spawn(io.read(move |_| {
        Ok((|| -> Result<String, crate::api::ApiError> {
            crate::api::current_metadata_admin(&worker_state, &worker_user, &worker_device)?;
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            crate::api::current_metadata_admin(&worker_state, &worker_user, &worker_device)?;
            Ok("PRIVATE_LOCAL_METADATA_NOT_PROVIDER_DATA".into())
        })())
    }));
    started.await.unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("UPDATE users SET role='member' WHERE id=?1", [&user.id])
        .unwrap();
    release.send(()).unwrap();
    let result = task.await.unwrap().unwrap();
    assert_eq!(result.unwrap_err().status, StatusCode::FORBIDDEN);
}

fn indexed_legacy_session(f: &Fixture, id: &str) -> tokio::sync::mpsc::Receiver<()> {
    let store = f.state.store.lock().unwrap();
    let (user, device) = store.authenticate(&f.token).unwrap().unwrap();
    store
        .create_session(
            id,
            &user.id,
            &f.project,
            "Private legacy session",
            &device.id,
        )
        .unwrap();
    assert!(
        store
            .save_transcript_if_revision(id, r#"["PRIVATE_LEGACY_TRANSCRIPT"]"#, 0)
            .unwrap()
    );
    let (close_tx, close_rx) = tokio::sync::mpsc::channel(1);
    f.state.leases.lock().unwrap().insert(
        id.into(),
        crate::api::Lease {
            device_id: device.id,
            connection_id: "legacy-fixture".into(),
            close_tx,
            expires_at: std::time::Instant::now() + Duration::from_secs(60),
        },
    );
    close_rx
}

#[tokio::test]
async fn legacy_session_owner_needs_current_project_grant_for_data_delete_and_takeover() {
    let f = Fixture::new();
    let mut close = indexed_legacy_session(&f, "native-legacy");
    let transcript = "/v1/sessions/native-legacy/transcript";
    for endpoint in [transcript, "/v1/sessions/native-legacy/transcript/revision"] {
        assert_eq!(
            f.get_json(endpoint, Some(&f.ungranted_token)).await.0,
            StatusCode::NOT_FOUND
        );
    }
    let owner = {
        let store = f.state.store.lock().unwrap();
        let owner = store.authenticate(&f.token).unwrap().unwrap().0;
        store.revoke_grant(&f.project, &owner.id).unwrap();
        owner.id
    };
    for endpoint in [
        transcript.to_string(),
        format!("{transcript}/revision"),
        format!("/v1/sessions?project={}", f.project),
        format!("/v1/project/settings?project={}", f.project),
        format!("/v1/context?project={}", f.project),
    ] {
        let (status, body) = f.get_json(&endpoint, Some(&f.token)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{endpoint}: {body}");
        assert!(!body.to_string().contains("PRIVATE_LEGACY_TRANSCRIPT"));
    }
    assert_eq!(
        f.put(
            transcript,
            Some(&f.token),
            json!({"payload":["must not save"],"expected_revision":1})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.delete("/v1/sessions/native-legacy", Some(&f.token)).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.post(
            "/v1/sessions/native-legacy/takeover",
            Some(&f.token),
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert!(matches!(
        close.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    assert!(f.state.leases.lock().unwrap().contains_key("native-legacy"));
    {
        let store = f.state.store.lock().unwrap();
        assert_eq!(store.transcript_revision("native-legacy").unwrap(), 1);
        assert!(
            store
                .load_transcript("native-legacy")
                .unwrap()
                .unwrap()
                .contains("PRIVATE_LEGACY_TRANSCRIPT")
        );
        store.grant(&f.project, &owner).unwrap();
    }
    assert_eq!(
        f.get_json(transcript, Some(&f.token)).await.0,
        StatusCode::OK,
        "Fresh requests may use explicitly renewed authority"
    );
}

#[tokio::test]
async fn disabled_legacy_transcript_storage_still_refuses_a_current_owners_write() {
    let mut f = Fixture::new();
    let _close = indexed_legacy_session(&f, "disabled-legacy");
    let config = crate::config::Config {
        store_transcripts: false,
        ..(*f.state.config).clone()
    };
    f.state.config = Arc::new(config);
    let (status, body) = f
        .put(
            "/v1/sessions/disabled-legacy/transcript",
            Some(&f.token),
            json!({"payload":["must not save"],"expected_revision":1}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.to_string().contains("disabled"));
    assert_eq!(
        f.state
            .store
            .lock()
            .unwrap()
            .transcript_revision("disabled-legacy")
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn legacy_pinned_session_proof_cannot_read_save_delete_or_takeover_after_atomic_regrant() {
    let f = Fixture::new();
    let mut close = indexed_legacy_session(&f, "aba-legacy");
    let (user, device) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    let io = crate::api::owned_session(&f.state, &user.id, &device.id, "aba-legacy").unwrap();
    revoke_and_regrant(&f);
    let called = AtomicUsize::new(0);
    let read = crate::api::with_owned_session(&f.state, &io, "aba-legacy", |store| {
        called.fetch_add(1, Ordering::SeqCst);
        Ok(store.load_transcript("aba-legacy")?)
    });
    assert_eq!(read.unwrap_err().status, StatusCode::FORBIDDEN);
    for operation in ["save", "delete", "takeover"] {
        let result = crate::api::with_owned_session(&f.state, &io, "aba-legacy", |store| {
            called.fetch_add(1, Ordering::SeqCst);
            match operation {
                "save" => {
                    store.save_transcript_if_revision("aba-legacy", r#"["must not save"]"#, 1)?;
                }
                "delete" => store.delete_session("aba-legacy")?,
                _ => {
                    f.state.leases.lock().unwrap().remove("aba-legacy");
                }
            }
            Ok(())
        });
        assert_eq!(
            result.unwrap_err().status,
            StatusCode::FORBIDDEN,
            "{operation}"
        );
    }
    assert_eq!(called.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.state
            .store
            .lock()
            .unwrap()
            .transcript_revision("aba-legacy")
            .unwrap(),
        1
    );
    assert!(f.state.leases.lock().unwrap().contains_key("aba-legacy"));
    assert!(matches!(
        close.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    // A proof for a different granted project still cannot be retargeted by ID.
    let other = {
        let store = f.state.store.lock().unwrap();
        let project = store
            .create_project(
                "other-legacy-project",
                f.root.join("other-project").to_str().unwrap(),
            )
            .unwrap();
        store.grant(&project.id, &user.id).unwrap();
        project
    };
    let wrong = NativeIo::capture(&f.state, &user.id, &device.id, &other.id).unwrap();
    assert_eq!(
        crate::api::with_owned_session(&f.state, &wrong, "aba-legacy", |_| Ok(()))
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn project_settings_proof_fences_aba_and_cached_admin_role_cannot_escalate_writes() {
    let f = Fixture::new();
    let (mut fake_admin, member_device) = f
        .state
        .store
        .lock()
        .unwrap()
        .authenticate(&f.token)
        .unwrap()
        .unwrap();
    fake_admin.role = "admin".into();
    let result = crate::api::project_settings_put(
        crate::api::AuthUser {
            user: fake_admin,
            device: member_device,
        },
        axum::extract::State(f.state.clone()),
        axum::Json(crate::api::ProjectSettingsBody {
            project: f.project.clone(),
            settings: Default::default(),
        }),
    )
    .await;
    assert_eq!(result.unwrap_err().status, StatusCode::FORBIDDEN);
    assert!(
        f.state
            .store
            .lock()
            .unwrap()
            .project_settings(&f.project)
            .unwrap()
            .is_none()
    );
    let (admin, device) = {
        let store = f.state.store.lock().unwrap();
        let user = store.create_user("settings-proof-admin", "admin").unwrap();
        store.grant(&f.project, &user.id).unwrap();
        let (device, _) = store.create_device(&user.id, "settings").unwrap();
        (user, device)
    };
    let io = NativeIo::capture(&f.state, &admin.id, &device.id, &f.project).unwrap();
    let initial = r#"{"environment":{"LANG":"initial"}}"#;
    crate::api::write_project_settings(&f.state, &io, initial).unwrap();
    {
        let store = f.state.store.lock().unwrap();
        store.revoke_grant(&f.project, &admin.id).unwrap();
        store.grant(&f.project, &admin.id).unwrap();
    }
    assert_eq!(
        crate::api::read_project_settings(&f.state, &io)
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        crate::api::write_project_settings(&f.state, &io, r#"{"environment":{"LANG":"stale"}}"#)
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.state
            .store
            .lock()
            .unwrap()
            .project_settings(&f.project)
            .unwrap()
            .as_deref(),
        Some(initial)
    );
    let fresh = NativeIo::capture(&f.state, &admin.id, &device.id, &f.project).unwrap();
    crate::api::write_project_settings(&f.state, &fresh, r#"{"environment":{"LANG":"fresh"}}"#)
        .unwrap();
}

#[tokio::test]
async fn settings_write_checks_current_admin_under_the_same_store_cut_as_the_update() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let store = crate::store::Store::open(&path).unwrap();
    let admin = store.create_user("settings-admin", "admin").unwrap();
    let (device, _) = store.create_device(&admin.id, "settings").unwrap();
    let project = store
        .create_project("settings", directory.path().to_str().unwrap())
        .unwrap();
    store.grant(&project.id, &admin.id).unwrap();
    let state = crate::api::AppState::new(
        store,
        crate::config::Config {
            db: path.clone(),
            sandbox: false,
            ..Default::default()
        },
    );
    let io = NativeIo::capture(&state, &admin.id, &device.id, &project.id).unwrap();
    crate::api::write_project_settings(&state, &io, "{}").unwrap();
    rusqlite::Connection::open(path)
        .unwrap()
        .execute("UPDATE users SET role='member' WHERE id=?1", [&admin.id])
        .unwrap();
    assert!(
        io.validate().is_ok(),
        "Ordinary native read authority alone does not confer administrator access"
    );
    assert_eq!(
        crate::api::write_project_settings(&state, &io, r#"{"environment":{"LANG":"forbidden"}}"#)
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        state
            .store
            .lock()
            .unwrap()
            .project_settings(&project.id)
            .unwrap()
            .as_deref(),
        Some("{}")
    );
}

#[test]
fn public_artifact_rechecks_its_token_capability_after_queued_io_without_a_native_grant_dependency()
{
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let f = Fixture::new();
        let row = upload_artifact(&f).await;
        let id = row["id"].as_str().unwrap();
        let (status, share) = f
            .post(
                &format!("/v1/artifacts/{id}/share"),
                Some(&f.token),
                json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        let permits = f.state.fs_reads.available_permits();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
        });
        started.await.unwrap();
        let request = Request::get(share["url"].as_str().unwrap())
            .body(Body::empty())
            .unwrap();
        let state = f.state.clone();
        let response = tokio::spawn(async move { router(state).oneshot(request).await.unwrap() });
        tokio::time::timeout(Duration::from_secs(3), async {
            while f.state.fs_reads.available_permits() == permits {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            f.delete(&format!("/v1/artifacts/{id}/share"), Some(&f.token))
                .await,
            StatusCode::NO_CONTENT
        );
        release.send(()).unwrap();
        blocker.await.unwrap();
        let response = response.await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("NATIVE_ARTIFACT_PRIVATE_MARKER"));
        assert_eq!(f.state.fs_reads.available_permits(), permits);
    });
}

#[test]
fn attachment_namespace_creation_revalidates_the_captured_grant() {
    let f = Fixture::new();
    let guard = scope(&f);
    revoke_and_regrant(&f);
    let error = super::super::safe_project_directory(
        f.root.to_str().unwrap(),
        ".dunlin/attachments",
        &guard,
    )
    .unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
    assert!(!f.root.join(".dunlin").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn attachment_filesystem_failure_preserves_internal_error_status() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    fs::set_permissions(&f.root, fs::Permissions::from_mode(0o500)).unwrap();
    let result = f
        .post(
            "/v1/fs/upload",
            Some(&f.token),
            json!({"project":f.project,"name":"fixture.txt","contents":"eA=="}),
        )
        .await;
    fs::set_permissions(&f.root, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(result.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!f.root.join(".dunlin").exists());
}
