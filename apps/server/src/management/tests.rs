use super::*;
#[test]
fn admin_authority_is_state_free_and_reloads_device_and_current_role() {
    let dir = tempfile::tempdir().unwrap();
    let store = crate::store::Store::open(dir.path().join("db")).unwrap();
    let user = store.create_user("admin", "admin").unwrap();
    let member = store.create_user("member", "member").unwrap();
    let (device, _) = store.create_device(&user.id, "admin-device").unwrap();
    let (member_device, _) = store.create_device(&member.id, "member-device").unwrap();
    let state = AppState::new(
        store,
        Config {
            db: dir.path().join("db"),
            ..Default::default()
        },
    );
    let proof = authority::AdminAuthority::capture(
        &state,
        &AuthUser {
            user: user.clone(),
            device: device.clone(),
        },
    )
    .unwrap();
    let mut forged = member;
    forged.role = "admin".into();
    assert!(
        authority::AdminAuthority::capture(
            &state,
            &AuthUser {
                user: forged,
                device: member_device
            }
        )
        .is_err()
    );
    state
        .store
        .lock()
        .unwrap()
        .revoke_device(&device.id)
        .unwrap();
    assert_eq!(
        proof.validate(&state).unwrap_err().status,
        StatusCode::UNAUTHORIZED
    );
}

fn fixture() -> (tempfile::TempDir, AppState, AuthUser) {
    let dir = tempfile::tempdir().unwrap();
    let store = crate::store::Store::open(dir.path().join("db")).unwrap();
    let user = store.create_user("publisher", "admin").unwrap();
    let (device, _) = store.create_device(&user.id, "publisher-device").unwrap();
    let state = AppState::new(
        store,
        Config {
            db: dir.path().join("db"),
            ..Default::default()
        },
    );
    (dir, state, AuthUser { user, device })
}
fn local_clone(stage: &Path) -> Result<PathBuf, String> {
    let repo = stage.join("owner/repo");
    std::fs::create_dir_all(&repo).map_err(|e| e.to_string())?;
    let status = std::process::Command::new("git")
        .args(["init", "-q"])
        .arg(&repo)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Local fixture Git init failed".into());
    }
    std::fs::write(repo.join("local-fixture"), b"no provider or credentials")
        .map_err(|e| e.to_string())?;
    Ok(repo)
}
#[test]
fn revoked_authorizing_device_during_private_clone_preparation_never_publishes_or_grants() {
    let (_dir, state, auth) = fixture();
    let proof = authority::AdminAuthority::capture(&state, &auth).unwrap();
    let worker_state = state.clone();
    let (entered, started) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let task = std::thread::spawn(move || {
        clone_in_stage(&worker_state, &proof, "owner/repo", |stage| {
            let repo = local_clone(stage)?;
            entered.send(()).map_err(|e| e.to_string())?;
            wait.recv_timeout(Duration::from_secs(3))
                .map_err(|e| e.to_string())?;
            Ok(repo)
        })
    });
    started.recv_timeout(Duration::from_secs(3)).unwrap();
    state
        .store
        .lock()
        .unwrap()
        .revoke_device(&auth.device.id)
        .unwrap();
    release.send(()).unwrap();
    let error = task.join().unwrap().unwrap_err();
    assert_eq!(error.status, StatusCode::UNAUTHORIZED);
    let projects = root(&state.config).join("projects");
    assert!(!projects.join("owner").exists());
    assert!(std::fs::read_dir(&projects).unwrap().next().is_none());
    let store = state.store.lock().unwrap();
    assert!(store.list_projects().unwrap().is_empty());
    assert!(store.projects_for_user(&auth.user.id).unwrap().is_empty());
}
#[test]
fn fresh_admin_can_publish_local_preparation_and_registration_failure_is_explicitly_uncertain() {
    let (_dir, state, auth) = fixture();
    let proof = authority::AdminAuthority::capture(&state, &auth).unwrap();
    let project = clone_in_stage(&state, &proof, "owner/repo", local_clone).unwrap();
    assert_eq!(
        project.path,
        root(&state.config)
            .join("projects/owner/repo")
            .to_str()
            .unwrap()
    );
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .project_for_user(&project.id, &auth.user.id)
            .unwrap()
            .is_some()
    );
    let (_dir, state, auth) = fixture();
    let proof = authority::AdminAuthority::capture(&state, &auth).unwrap();
    state
        .store
        .lock()
        .unwrap()
        .create_project("owner-repo", "/existing-unrelated-project")
        .unwrap();
    let error = clone_in_stage(&state, &proof, "owner/repo", local_clone).unwrap_err();
    assert_eq!(error.status, StatusCode::CONFLICT);
    assert!(error.message.contains("partially completed"));
    assert!(error.message.contains("do not automatically retry"));
    assert!(
        root(&state.config)
            .join("projects/owner/repo/local-fixture")
            .exists()
    );
    let store = state.store.lock().unwrap();
    assert_eq!(store.list_projects().unwrap().len(), 1);
    assert!(store.projects_for_user(&auth.user.id).unwrap().is_empty());
}

fn recorded_job(
    state: &AppState,
    authority: &authority::AdminAuthority,
) -> (Arc<Mutex<Job>>, Arc<std::sync::atomic::AtomicBool>) {
    let job = Arc::new(Mutex::new(Job {
        id: "local-operation".into(),
        kind: "local-fixture".into(),
        output: String::new(),
        finished: false,
        success: false,
    }));
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    state.management.lock().unwrap().jobs.insert(
        "local-operation".into(),
        Running {
            owner: authority.user.clone(),
            authority: authority.clone(),
            require_admin_only_relay: false,
            created: Instant::now(),
            job: job.clone(),
            cancel: cancel.clone(),
        },
    );
    (job, cancel)
}

#[tokio::test]
async fn management_output_cancel_and_settings_require_current_originating_device() {
    let (_dir, state, auth) = fixture();
    let proof = authority::AdminAuthority::capture(&state, &auth).unwrap();
    let (shared, cancel_flag) = recorded_job(&state, &proof);
    shared.lock().unwrap().output = "private local operation output".into();
    let other = {
        let store = state.store.lock().unwrap();
        let (device, _) = store
            .create_device(&auth.user.id, "other-active-device")
            .unwrap();
        AuthUser {
            user: auth.user.clone(),
            device,
        }
    };
    assert_eq!(
        job(
            copy_auth(&other),
            State(state.clone()),
            RoutePath("local-operation".into())
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        cancel(
            other,
            State(state.clone()),
            RoutePath("local-operation".into())
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::NOT_FOUND
    );
    assert!(!cancel_flag.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(
        job(
            copy_auth(&auth),
            State(state.clone()),
            RoutePath("local-operation".into())
        )
        .await
        .unwrap()
        .0
        .output,
        "private local operation output"
    );
    state
        .store
        .lock()
        .unwrap()
        .revoke_device(&auth.device.id)
        .unwrap();
    assert_eq!(
        job(
            copy_auth(&auth),
            State(state.clone()),
            RoutePath("local-operation".into())
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        cancel(
            copy_auth(&auth),
            State(state.clone()),
            RoutePath("local-operation".into())
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        info(copy_auth(&auth), State(state.clone()))
            .await
            .unwrap_err()
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        update(
            auth,
            State(state.clone()),
            Json(Update {
                name: Some("forbidden publication".into()),
                harness: None
            })
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::UNAUTHORIZED
    );
    assert!(!root(&state.config).join("settings.json").exists());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_device_stops_server_owned_local_operation_and_its_process_group() {
    assert_local_operation_stops(false).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn member_added_after_host_login_start_stops_the_controlled_process_group() {
    assert_local_operation_stops(true).await;
}

#[cfg(unix)]
async fn assert_local_operation_stops(member_added: bool) {
    let (dir, state, auth) = fixture();
    let proof = authority::AdminAuthority::capture(&state, &auth).unwrap();
    let (shared, cancel) = recorded_job(&state, &proof);
    state
        .management
        .lock()
        .unwrap()
        .jobs
        .get_mut("local-operation")
        .unwrap()
        .require_admin_only_relay = member_added;
    let parent_pid = dir.path().join("parent.pid");
    let child_pid = dir.path().join("child.pid");
    let late_effect = dir.path().join("forbidden-late-effect");
    let mut command = tokio::process::Command::new("/bin/sh");
    command
        .args([
            "-c",
            "echo $$ > \"$1\"; /bin/sh -c 'sleep 1; : > \"$1\"; sleep 30' local-child \"$3\" & echo $! > \"$2\"; printf 'local-started\\n'; wait",
            "local-fixture",
        ])
        .arg(&parent_pid)
        .arg(&child_pid)
        .arg(&late_effect);
    let task = tokio::spawn(run(
        state.clone(),
        proof,
        member_added,
        command,
        shared.clone(),
        cancel,
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !child_pid.is_file() || !shared.lock().unwrap().output.contains("local-started") {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    if member_added {
        state
            .store
            .lock()
            .unwrap()
            .create_user("new-member", "member")
            .unwrap();
        assert_eq!(
            job(
                copy_auth(&auth),
                State(state.clone()),
                RoutePath("local-operation".into())
            )
            .await
            .unwrap_err()
            .status,
            StatusCode::CONFLICT
        );
        assert_eq!(
            super::cancel(
                copy_auth(&auth),
                State(state.clone()),
                RoutePath("local-operation".into())
            )
            .await
            .unwrap_err()
            .status,
            StatusCode::CONFLICT
        );
    } else {
        state
            .store
            .lock()
            .unwrap()
            .revoke_device(&auth.device.id)
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    let result = shared.lock().unwrap().clone();
    assert!(result.finished);
    assert!(!result.success);
    assert!(result.output.contains(OPERATION_UNKNOWN));
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        !late_effect.exists(),
        "Revoked process group performed a late effect"
    );
    for path in [parent_pid, child_pid] {
        let pid: i32 = std::fs::read_to_string(path)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if unsafe { libc::kill(pid, 0) } != 0 {
                    break;
                }
                #[cfg(target_os = "linux")]
                if std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
                    stat.rsplit_once(')')
                        .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn stale_cached_admin_never_spawns_and_fresh_local_job_keeps_bounded_output() {
    let (dir, state, auth) = fixture();
    let member = {
        let store = state.store.lock().unwrap();
        let mut user = store.create_user("stale-claim", "member").unwrap();
        let (device, _) = store.create_device(&user.id, "forged-admin").unwrap();
        user.role = "admin".into();
        AuthUser { user, device }
    };
    assert_eq!(
        start(
            member,
            State(state.clone()),
            Json(Action {
                action: "status".into(),
                target: "github".into()
            })
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::FORBIDDEN
    );
    assert!(state.management.lock().unwrap().jobs.is_empty());
    let proof = authority::AdminAuthority::capture(&state, &auth).unwrap();
    let (shared, cancel) = recorded_job(&state, &proof);
    let mut command = tokio::process::Command::new("/bin/sh");
    command.args([
        "-c",
        "i=0; while [ $i -lt 4000 ]; do printf 'bounded-output-line\\n'; i=$((i+1)); done",
    ]);
    tokio::time::timeout(
        Duration::from_secs(10),
        run(
            state.clone(),
            proof.clone(),
            false,
            command,
            shared.clone(),
            cancel,
        ),
    )
    .await
    .unwrap();
    let result = shared.lock().unwrap().clone();
    assert!(result.finished && result.success);
    assert!(result.output.len() <= 32768);
    let marker = dir.path().join("must-not-start");
    state
        .store
        .lock()
        .unwrap()
        .revoke_device(&auth.device.id)
        .unwrap();
    let mut command = tokio::process::Command::new("/bin/sh");
    command
        .args(["-c", "printf forbidden > \"$1\"", "local-fixture"])
        .arg(&marker);
    let (shared, cancel) = recorded_job(&state, &proof);
    run(state, proof, false, command, shared.clone(), cancel).await;
    assert!(!marker.exists());
    assert!(!shared.lock().unwrap().success);
}

fn copy_auth(auth: &AuthUser) -> AuthUser {
    AuthUser {
        user: auth.user.clone(),
        device: auth.device.clone(),
    }
}

#[cfg(unix)]
#[tokio::test]
async fn operation_observes_exit_without_reaping_before_group_cleanup() {
    let mut command = tokio::process::Command::new("/bin/sh");
    command
        .args(["-c", "exit 0"])
        .process_group(0)
        .kill_on_drop(true);
    let child = command.spawn().unwrap();
    let pid = child.id().unwrap();
    let group = Arc::new(OperationGroup {
        pid,
        terminated: Mutex::new(false),
    });
    let mut process = OperationChild {
        child,
        group: group.clone(),
    };
    assert!(
        tokio::time::timeout(Duration::from_secs(5), process.completion())
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    // Multiple observers can still see this exited child: completion did not
    // reap it and leave the raw process/group ID available for reuse.
    assert!(operation_exit_pending(pid).unwrap());
    assert!(operation_exit_pending(pid).unwrap());
    assert!(process.finish().await.unwrap().success());
    assert!(operation_exit_pending(pid).is_err());
    assert!(*group.terminated.lock().unwrap());
    group.terminate(); // Guardian/owner cleanup after reap must remain a no-op.
}
