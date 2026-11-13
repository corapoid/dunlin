use super::*;
#[cfg(target_os = "linux")]
use crate::store::local_metadata::MetadataPermission;
use std::fs;

fn request() -> CreateRequest {
    CreateRequest::Legacy(
        Options {
            model: "fixture-model".into(),
            mode: "plan".into(),
            max_seconds: 10,
            max_output_bytes: 1024 * 1024,
        }
        .into(),
    )
}
fn native_id(f: &Fixture) -> String {
    f.state
        .store
        .lock()
        .unwrap()
        .local_binding_for_user(&f.auth.user.id, &f.workspace, &f.project)
        .unwrap()
        .unwrap()
        .native_project_id
}
fn bind(f: &Fixture, native: &str, metadata: Option<u64>) {
    let prepared = {
        let store = f.state.store.lock().unwrap();
        let revision = store
            .local_binding_for_user(&f.auth.user.id, &f.workspace, &f.project)
            .unwrap()
            .map(|b| b.revision);
        store
            .prepare_local_binding(
                &f.auth.user.id,
                &f.workspace,
                &f.project,
                native,
                revision,
                metadata,
            )
            .unwrap()
    }
    .validate_filesystem()
    .unwrap();
    f.state
        .store
        .lock()
        .unwrap()
        .commit_local_binding_authorized(prepared, |_, _| Ok::<_, crate::store::StoreError>(()))
        .unwrap();
}
#[cfg(target_os = "linux")]
fn linked(f: &Fixture, permissions: &[MetadataPermission]) -> String {
    let native = native_id(f);
    let root = f.dir.path().join("checkout");
    let common = f.dir.path().join("repository/.git");
    let git = common.join("worktrees/linked");
    fs::create_dir_all(&git).unwrap();
    fs::create_dir_all(common.join("objects")).unwrap();
    fs::write(root.join(".git"), format!("gitdir: {}\n", git.display())).unwrap();
    fs::write(
        git.join("gitdir"),
        format!("{}\n", root.join(".git").display()),
    )
    .unwrap();
    fs::write(git.join("commondir"), "../..\n").unwrap();
    fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(common.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    let preparation = f
        .state
        .store
        .lock()
        .unwrap()
        .prepare_local_metadata(&native)
        .unwrap();
    let prepared = preparation.capture(&git, &common).unwrap();
    {
        let store = f.state.store.lock().unwrap();
        let approval = store.commit_local_metadata(prepared, None).unwrap();
        let grant = store
            .capture_native_grant(&native, &f.auth.user.id)
            .unwrap();
        store
            .set_local_metadata_grant(
                &native,
                &f.auth.user.id,
                approval.revision,
                grant.revision,
                permissions,
                true,
                None,
                permissions.contains(&MetadataPermission::Write),
            )
            .unwrap();
    }
    bind(f, &native, Some(1));
    native
}

#[test]
fn prepared_admission_cannot_adopt_a_new_native_grant_epoch_at_the_final_sql_cut() {
    let f = fixture("normal");
    let request = request();
    let prepared =
        prepare_create_scope(&f.state, &f.auth, &f.workspace, &f.project, &request).unwrap();
    let native = native_id(&f);
    let store = f.state.store.lock().unwrap();
    store.revoke_grant(&native, &f.auth.user.id).unwrap();
    store.grant(&native, &f.auth.user.id).unwrap();
    let result = store.native_checkout_transaction(|store| {
        let fresh = revalidate_create_scope(
            &f.state,
            store,
            &f.auth,
            &f.workspace,
            &f.project,
            &request,
            &prepared,
        )?;
        store
            .interactive_journal()
            .create_admitted_profile_in_transaction(
                &fresh.authority,
                &f.workspace,
                &f.project,
                &f.auth.device.id,
                "stale-checkout-intent-0001",
                &prepared.scope.fingerprint,
                "stale-actor",
                None,
            )
            .map_err(|e| CreateCutError(admission_error(e)))
    });
    assert!(result.is_err());
    assert!(store.ahp_summaries(&f.auth.user.id).unwrap().is_empty());
    assert!(
        store
            .interactive_journal()
            .admission_lookup(
                &prepared.scope.authority,
                &f.auth.device.id,
                "stale-checkout-intent-0001",
                &prepared.scope.fingerprint,
            )
            .unwrap()
            .is_none()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn aliased_ordinary_checkout_replays_the_historical_canonical_path_fingerprint() {
    let f = fixture("normal");
    let alias = f.dir.path().join("checkout-alias");
    std::os::unix::fs::symlink(f.dir.path().join("checkout"), &alias).unwrap();
    let native = {
        let store = f.state.store.lock().unwrap();
        let native = store
            .create_project("alias", alias.to_str().unwrap())
            .unwrap();
        store.grant(&native.id, &f.auth.user.id).unwrap();
        native
    };
    bind(&f, &native.id, None);
    let key = "historical-alias-intent-0001";
    let old = {
        let store = f.state.store.lock().unwrap();
        let (binding, resolved) = store
            .resolve_local_binding(
                &f.auth.user.id,
                &f.workspace,
                &f.project,
                Permission::CreateSession,
            )
            .unwrap()
            .unwrap();
        assert_ne!(native.path, resolved.path);
        let context = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        let stamp = context
            .authorize_project(&f.workspace, &f.project, Permission::CreateSession)
            .unwrap();
        let CreateRequest::Legacy(request) = request() else {
            unreachable!()
        };
        let old_identity = json!({
            "workspace":f.workspace,"project":f.project,"options":request.options(),
            "authority":stamp,"binding":binding,
            "nativeGrant":store.capture_native_grant(&native.id,&f.auth.user.id).unwrap(),
            "nativePath":resolved.path,"harness":crate::management::harness(&f.state.config),
        });
        let fingerprint = hex::encode(Sha256::digest(serde_json::to_vec(&old_identity).unwrap()));
        store
            .interactive_journal()
            .create_admitted(
                &context,
                &f.workspace,
                &f.project,
                &f.auth.device.id,
                key,
                &fingerprint,
                "historical-actor",
            )
            .unwrap()
            .session
    };
    let retry = Service::create_request_with_key(
        f.state.clone(),
        AuthUser {
            user: f.auth.user.clone(),
            device: f.auth.device.clone(),
        },
        f.workspace.clone(),
        f.project.clone(),
        request(),
        Some(key.into()),
    )
    .unwrap();
    assert!(retry.replayed);
    assert_eq!(
        serde_json::to_value(retry.session).unwrap(),
        serde_json::to_value(old).unwrap()
    );
    assert!(f.state.interactive.actors.lock().unwrap().is_empty());
    assert!(!f.dir.path().join("checkout/actor-starts").exists());
}

#[tokio::test]
async fn ordinary_objects_directory_replacement_stops_the_pinned_actor_without_forwarding_queue() {
    let f = fixture("normal");
    let native = native_id(&f);
    let git = f.dir.path().join("checkout/.git");
    fs::create_dir_all(git.join("objects")).unwrap();
    fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    bind(&f, &native, None);
    let session = start(&f);
    enqueue(&f, &session, "before-objects", "before objects change");
    until(&f, || {
        fs::read_to_string(f.dir.path().join("checkout/received-prompts"))
            .is_ok_and(|text| text == "before objects change\n")
    })
    .await;
    let pid = actor_pid(&f);
    fs::rename(git.join("objects"), git.join("retired-objects")).unwrap();
    fs::create_dir(git.join("objects")).unwrap();
    enqueue(
        &f,
        &session,
        "after-objects",
        "must not reach changed objects",
    );
    until(&f, || phase(&f, &session) == SessionPhase::Interrupted).await;
    until(&f, || !actor_process_running(pid)).await;
    assert_eq!(
        fs::read_to_string(f.dir.path().join("checkout/received-prompts")).unwrap(),
        "before objects change\n"
    );
}

// These assertions execute the actual production sandbox adapter, not an
// unsandboxed stand-in. The dedicated runner provisions real bubblewrap.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires dedicated real bubblewrap AppArmor/seccomp runtime runner"]
async fn production_browser_acceptance_runtime_linked_interactive() {
    assert!(std::path::Path::new("/usr/bin/bwrap").is_file());
    let mut f = permission_fixture("normal");
    let peer = f.dir.path().join("checkout/permission-peer.py");
    fs::write(
        &peer,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/acp/permission_peer.py"
        )),
    )
    .unwrap();
    Arc::get_mut(&mut f.state.config).unwrap().sandbox = true;
    Arc::get_mut(&mut f.state.interactive).unwrap().command = Some((
        "/usr/bin/python3".into(),
        vec!["-u".into(), peer.to_str().unwrap().into(), "normal".into()],
    ));
    let native = linked(&f, &[MetadataPermission::Read]);
    let create = |key: &str| {
        Service::create_request_with_key(
            f.state.clone(),
            AuthUser {
                user: f.auth.user.clone(),
                device: f.auth.device.clone(),
            },
            f.workspace.clone(),
            f.project.clone(),
            request(),
            Some(key.into()),
        )
    };
    assert!(create("linked-read-only-intent-0001").is_err());
    assert!(!f.dir.path().join("checkout/actor-pid").exists());
    let native_revision = {
        let store = f.state.store.lock().unwrap();
        let native_revision = store
            .capture_native_grant(&native, &f.auth.user.id)
            .unwrap()
            .revision;
        store
            .set_local_metadata_grant(
                &native,
                &f.auth.user.id,
                1,
                native_revision,
                &[MetadataPermission::Read, MetadataPermission::Write],
                true,
                Some(1),
                true,
            )
            .unwrap();
        native_revision
    };
    let accepted = create("linked-write-intent-0001").unwrap();
    let session = accepted.session;
    enqueue(
        &f,
        &session,
        "linked-old-permission",
        "old linked permission",
    );
    until(&f, || permission_action(&f, &session).is_some()).await;
    let action = permission_action(&f, &session).unwrap();
    let actor = sandbox_actor(&peer).expect("Actual sandboxed fixture Python must exist");
    {
        // Accept the human decision first, then revoke/regrant before the
        // actor can consume it. An active boolean alone would miss this ABA.
        let store = f.state.store.lock().unwrap();
        let authority = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        assert!(
            store
                .interactive_journal()
                .dispatch_action(
                    &authority,
                    &session.id,
                    &f.auth.device.id,
                    "linked-old-decision",
                    &action,
                    dunlin_session_runtime::DispatchAdmission::default(),
                )
                .unwrap()
                .rejection_reason
                .is_none()
        );
        store
            .revoke_local_metadata_grant(&native, &f.auth.user.id, 2)
            .unwrap();
        store
            .set_local_metadata_grant(
                &native,
                &f.auth.user.id,
                1,
                native_revision,
                &[MetadataPermission::Read, MetadataPermission::Write],
                true,
                Some(3),
                true,
            )
            .unwrap();
        assert!(!f.state.interactive.queue_text_allowed(
            &f.state,
            &store,
            &session.id,
            &f.auth.user.id
        ));
        assert_eq!(
            store
                .capture_native_grant(&native, &f.auth.user.id)
                .unwrap()
                .revision,
            native_revision
        );
    }
    until(&f, || phase(&f, &session) == SessionPhase::Interrupted).await;
    until(&f, || !sandbox_actor_running(&actor)).await;
    assert!(!f.dir.path().join("checkout/received-decisions").exists());
    assert!(!f.dir.path().join("checkout/allowed.txt").exists());
    assert_eq!(
        create("linked-write-intent-0001").unwrap_err(),
        "idempotency_conflict"
    );
    let fresh = create("linked-fresh-intent-0001").unwrap().session;
    enqueue(
        &f,
        &fresh,
        "linked-fresh-permission",
        "fresh linked permission",
    );
    until(&f, || permission_action(&f, &fresh).is_some()).await;
    let action = permission_action(&f, &fresh).unwrap();
    {
        let store = f.state.store.lock().unwrap();
        let authority = store
            .team_project_context(&f.workspace, &f.project, &f.auth.user.id)
            .unwrap();
        assert!(
            store
                .interactive_journal()
                .dispatch_action(
                    &authority,
                    &fresh.id,
                    &f.auth.device.id,
                    "linked-fresh-decision",
                    &action,
                    dunlin_session_runtime::DispatchAdmission::default(),
                )
                .unwrap()
                .rejection_reason
                .is_none()
        );
    }
    until(&f, || {
        fs::read_to_string(f.dir.path().join("checkout/allowed.txt"))
            .is_ok_and(|text| text == "fresh linked permission")
    })
    .await;
    stop(&f, &fresh).await;
}

/// A sandboxed PID file names a PID in the new namespace. Use only descendants
/// of this test process and exact fixture argv to identify its host process.
#[cfg(target_os = "linux")]
struct SandboxActor {
    pid: u32,
    start: String,
}
#[cfg(target_os = "linux")]
fn process_text(path: &std::path::Path) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!(
            "Cannot inspect sandbox fixture process at {}: {error}",
            path.display()
        ),
    }
}
#[cfg(target_os = "linux")]
fn sandbox_actor_identity(pid: u32) -> Option<SandboxActor> {
    let stat = process_text(&std::path::PathBuf::from(format!("/proc/{pid}/stat")))?;
    let (prefix, tail) = stat
        .rsplit_once(") ")
        .unwrap_or_else(|| panic!("Malformed stat for fixture PID {pid}"));
    let recorded = prefix
        .split_once(" (")
        .and_then(|(number, _)| number.parse::<u32>().ok())
        .unwrap_or_else(|| panic!("Malformed stat PID/comm for fixture PID {pid}"));
    assert_eq!(recorded, pid, "Mismatched fixture stat PID");
    let fields = tail.split_whitespace().collect::<Vec<_>>();
    assert!(fields.len() >= 20, "Truncated stat for fixture PID {pid}");
    assert!(
        fields[19].parse::<u64>().is_ok(),
        "Malformed start ticks for fixture PID {pid}"
    );
    if matches!(fields[0], "Z" | "X") {
        return None;
    }
    Some(SandboxActor {
        pid,
        start: fields[19].into(),
    })
}
#[cfg(target_os = "linux")]
fn sandbox_actor_running(actor: &SandboxActor) -> bool {
    sandbox_actor_identity(actor.pid).is_some_and(|current| current.start == actor.start)
}
#[cfg(target_os = "linux")]
fn sandbox_actor(peer: &std::path::Path) -> Option<SandboxActor> {
    let mut pending = vec![std::process::id()];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(pid) = pending.pop() {
        if !seen.insert(pid) {
            continue;
        }
        assert!(
            seen.len() <= 256,
            "Unexpected sandbox fixture descendant count"
        );
        let cmdline = match fs::read(format!("/proc/{pid}/cmdline")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("Cannot inspect fixture PID {pid}: {error}"),
        };
        let argv = cmdline.split(|byte| *byte == 0).collect::<Vec<_>>();
        if argv.first() == Some(&b"/usr/bin/python3".as_slice())
            && argv.contains(&peer.as_os_str().as_encoded_bytes())
            && let Some(actor) = sandbox_actor_identity(pid)
        {
            return Some(actor);
        }
        let tasks = match fs::read_dir(format!("/proc/{pid}/task")) {
            Ok(tasks) => tasks,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("Cannot inspect fixture PID {pid} tasks: {error}"),
        };
        for task in tasks {
            let task = task.expect("Cannot enumerate sandbox fixture process tasks");
            if let Some(children) = process_text(&task.path().join("children")) {
                pending.extend(children.split_whitespace().map(|value| {
                    value
                        .parse::<u32>()
                        .expect("Malformed sandbox fixture child PID")
                }));
            }
        }
    }
    None
}
