//! Model/profile tests do not claim kernel isolation. The ignored runtime test
//! requires the dedicated AppArmor/seccomp runner and executes real bubblewrap.
use super::*;
#[cfg(target_os = "linux")]
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message as ClientMessage, client::IntoClientRequest},
};
const PEER: &str = r#"
import json, os, subprocess, sys
child = subprocess.Popen([sys.executable, '-u', '-c', "import time\nwhile True:\n open('child-ticks','a').write('tick\\n')\n time.sleep(.05)\n"])
with open('bridge-started','a') as f: f.write('started\n')
for raw in sys.stdin:
 v=json.loads(raw)
 print(json.dumps({'jsonrpc':'2.0','id':v.get('id'),'result':{'echo':v.get('params',{}),'metadata':os.path.isfile('.git')}}),flush=True)
"#;
struct Fixture {
    _dir: tempfile::TempDir,
    state: AppState,
    user: User,
    device: Device,
    #[cfg(target_os = "linux")]
    token: String,
    project: Project,
    source: PathBuf,
    root: PathBuf,
    git: PathBuf,
    common: PathBuf,
}
fn git(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=Metadata fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "Local Git fixture: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let root = dir.path().join("checkout");
        std::fs::create_dir(&source).unwrap();
        git(&source, &["init", "-q"]);
        std::fs::write(source.join("README"), "fixture").unwrap();
        git(&source, &["add", "README"]);
        git(&source, &["commit", "-qm", "fixture"]);
        git(
            &source,
            &[
                "worktree",
                "add",
                "--detach",
                root.to_str().unwrap(),
                "HEAD",
            ],
        );
        std::fs::write(
            source.join("source-only-secret"),
            "private-source-workspace",
        )
        .unwrap();
        let git = PathBuf::from(git(&root, &["rev-parse", "--absolute-git-dir"]));
        let common = source.join(".git").canonicalize().unwrap();
        std::fs::write(root.join("peer.py"), PEER).unwrap();
        let store = crate::store::Store::open(dir.path().join("relay.db")).unwrap();
        let user = store.create_user("metadata-bridge", "member").unwrap();
        let (device, _token) = store.create_device(&user.id, "metadata").unwrap();
        let project = store
            .create_project("linked", root.to_str().unwrap())
            .unwrap();
        store.grant(&project.id, &user.id).unwrap();
        let prep = store
            .prepare_local_metadata(&project.id)
            .unwrap()
            .capture(&git, &common)
            .unwrap();
        store.commit_local_metadata(prep, None).unwrap();
        let state = AppState::new(
            store,
            crate::config::Config {
                db: dir.path().join("relay.db"),
                sandbox: true,
                harness: format!("/usr/bin/python3 -u {}", root.join("peer.py").display()),
                ..Default::default()
            },
        );
        Self {
            _dir: dir,
            state,
            user,
            device,
            #[cfg(target_os = "linux")]
            token: _token,
            project,
            source,
            root,
            git,
            common,
        }
    }
    fn grant(&self, permissions: &[MetadataPermission], expected: Option<u64>) {
        self.state
            .store
            .lock()
            .unwrap()
            .set_local_metadata_grant(
                &self.project.id,
                &self.user.id,
                1,
                1,
                permissions,
                true,
                expected,
                permissions.contains(&MetadataPermission::Write),
            )
            .unwrap();
    }
    fn authority(&self, permission: MetadataPermission) -> NativeCheckoutAuthority {
        let candidate = self
            .state
            .store
            .lock()
            .unwrap()
            .native_checkout_candidate(&self.project.id, &self.user.id, permission)
            .unwrap();
        let validated = candidate.validate_filesystem().unwrap();
        self.state
            .store
            .lock()
            .unwrap()
            .authorize_native_checkout(validated)
            .unwrap()
    }
    fn tracker(&self) -> Tracker {
        let a = self.authority(MetadataPermission::Write);
        let g = self
            .state
            .store
            .lock()
            .unwrap()
            .capture_native_grant(&self.project.id, &self.user.id)
            .unwrap();
        Tracker {
            native_projects: vec![self.project.clone()],
            native_grants: vec![g],
            native_checkouts: vec![a],
            allowed_roots: vec![self.root.to_str().unwrap().into()],
            ..Default::default()
        }
    }
}
#[test]
fn approved_metadata_is_separate_from_acp_workspace_directories_and_profiles() {
    let f = Fixture::new();
    assert!(
        f.state
            .store
            .lock()
            .unwrap()
            .native_checkout_candidate(&f.project.id, &f.user.id, MetadataPermission::Write)
            .is_err()
    );
    f.grant(&[MetadataPermission::Read], None);
    assert!(
        f.state
            .store
            .lock()
            .unwrap()
            .native_checkout_candidate(&f.project.id, &f.user.id, MetadataPermission::Write)
            .is_err()
    );
    f.grant(
        &[MetadataPermission::Read, MetadataPermission::Write],
        Some(1),
    );
    let mut tracker = f.tracker();
    assert!(tracker.roots_authorized(&f.state, &f.user));
    for metadata in [&f.git, &f.common] {
        let request = json!({"jsonrpc":"2.0","id":1,"method":"session/new","params":{"cwd":f.root,"additionalDirectories":[metadata],"mcpServers":[]}});
        let (close, _) = mpsc::channel(1);
        let error = tracker
            .on_client_message(
                &f.state,
                &f.user,
                &f.device,
                &f.project,
                "metadata-scope",
                &close,
                &request,
            )
            .unwrap_err();
        assert_eq!(error["error"]["code"], INVALID_PARAMS);
    }
    assert_eq!(
        tracker.allowed_roots,
        vec![f.root.to_str().unwrap().to_owned()]
    );
    assert!(tracker.allowed_project_ids.is_empty());
    assert!(tracker.vault_sessions.is_empty());
    assert!(
        !tracker
            .allowed_roots
            .contains(&f.common.to_str().unwrap().to_owned())
    );
    let read = f.authority(MetadataPermission::Read).metadata_roots();
    assert!(metadata_roots_safe(&f.state.config, &read));
    let mut unsandboxed = (*f.state.config).clone();
    unsandboxed.sandbox = false;
    assert!(!metadata_roots_safe(&unsandboxed, &read));
    let profile = sandbox_profile_with_metadata(
        f.root.to_str().unwrap(),
        &[],
        &[f.source.to_str().unwrap().into()],
        "python3",
        &f.state.config.db,
        &read,
    );
    assert!(profile.contains(&format!(
        "(deny file-write* (subpath \"{}\"))",
        f.common.display()
    )));
    assert!(profile.contains(&format!(
        "(require-not (subpath \"{}\"))",
        f.common.display()
    )));
}
#[test]
fn metadata_mount_cannot_cover_another_registered_workspace_or_its_alias() {
    let f = Fixture::new();
    f.grant(&[MetadataPermission::Read], None);
    let roots = f.authority(MetadataPermission::Read).metadata_roots();
    let descendant = f.common.join("separate-workspace");
    std::fs::create_dir(&descendant).unwrap();
    let mut others = vec![descendant.clone()];
    #[cfg(unix)]
    {
        let alias = f._dir.path().join("workspace-alias");
        std::os::unix::fs::symlink(&descendant, &alias).unwrap();
        others.push(alias);
    }
    for other in others {
        let command = harness_command_with_brokers(
            &f.state.config,
            "python3",
            &[],
            f.root.to_str().unwrap(),
            &[],
            &[other.to_str().unwrap().into()],
            &ProjectSettings::default(),
            &f.user.id,
            &[],
            &roots,
        );
        assert_eq!(
            command.as_std().get_program(),
            "/nonexistent/dunlin-metadata-sandbox-scope-denied"
        );
    }
}

#[cfg(target_os = "linux")]
struct Server(tokio::task::JoinHandle<()>);
#[cfg(target_os = "linux")]
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}
#[cfg(target_os = "linux")]
async fn connect(
    f: &Fixture,
) -> (
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Server,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "ws://{}/acp?project={}",
        listener.local_addr().unwrap(),
        f.project.id
    );
    let app = crate::api::router(f.state.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", f.token).parse().unwrap(),
    );
    (connect_async(request).await.unwrap().0, Server(task))
}
#[cfg(target_os = "linux")]
async fn closed(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while let Some(frame) = socket.next().await {
            match frame {
                Ok(ClientMessage::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    })
    .await
    .expect("Metadata guardian must close bridge");
}
#[cfg(target_os = "linux")]
async fn kernel_probe(f: &Fixture, permission: MetadataPermission) {
    let code = r#"import json,os,sys
common,source,mode=sys.argv[1:]
assert open(os.path.join(common,'HEAD')).read()
try: open(os.path.join(source,'source-only-secret')).read()
except OSError: pass
else: raise AssertionError('unapproved source workspace was readable')
p=os.path.join(common,'fixture-'+mode)
try: open(p,'w').write('probe')
except OSError:
 assert mode=='read'
else:
 assert mode=='write'
print('KERNEL_METADATA_OK')
"#;
    let roots = f.authority(permission).metadata_roots();
    let mode = if permission == MetadataPermission::Write {
        "write"
    } else {
        "read"
    };
    let args = vec![
        "-c".into(),
        code.into(),
        f.common.to_str().unwrap().into(),
        f.source.to_str().unwrap().into(),
        mode.into(),
    ];
    let mut cmd = harness_command_with_brokers(
        &f.state.config,
        "/usr/bin/python3",
        &args,
        f.root.to_str().unwrap(),
        &[],
        &[f.source.to_str().unwrap().into()],
        &ProjectSettings::default(),
        &f.user.id,
        &[],
        &roots,
    );
    cmd.current_dir(&f.root)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(10), cmd.output())
        .await
        .expect("Kernel probe deadline")
        .unwrap();
    assert!(
        out.status.success(),
        "Actual bubblewrap probe failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("KERNEL_METADATA_OK"));
}
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires dedicated real bubblewrap AppArmor/seccomp runtime runner"]
async fn production_browser_acceptance_runtime_metadata() {
    for change in ["revoke", "objects", "alias"] {
        let mut f = Fixture::new();
        let (mut denied, _server) = connect(&f).await;
        closed(&mut denied).await;
        assert!(!f.root.join("bridge-started").exists());
        f.grant(&[MetadataPermission::Read], None);
        let (mut denied, _server) = connect(&f).await;
        closed(&mut denied).await;
        assert!(!f.root.join("bridge-started").exists());
        f.grant(
            &[MetadataPermission::Read, MetadataPermission::Write],
            Some(1),
        );
        kernel_probe(&f, MetadataPermission::Read).await;
        kernel_probe(&f, MetadataPermission::Write).await;
        let (mut socket, _server) = connect(&f).await;
        socket.send(ClientMessage::Text(json!({"jsonrpc":"2.0","id":10,"method":"fixture/probe","params":{"before":"revoke"}}).to_string().into())).await.unwrap();
        let response = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(Ok(ClientMessage::Text(text))) = socket.next().await {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if value["id"] == 10 {
                        break value;
                    }
                }
            }
        })
        .await
        .expect("Real sandboxed bridge must reply");
        assert_eq!(response["result"]["echo"]["before"], "revoke");
        tokio::time::timeout(Duration::from_secs(5), async {
            while std::fs::read_to_string(f.root.join("child-ticks"))
                .unwrap_or_default()
                .lines()
                .count()
                < 2
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        match change {
            "revoke" => {
                f.state
                    .store
                    .lock()
                    .unwrap()
                    .revoke_local_metadata_grant(&f.project.id, &f.user.id, 2)
                    .unwrap();
            }
            "objects" => {
                std::fs::rename(f.common.join("objects"), f.common.join("old-objects")).unwrap();
                std::fs::create_dir(f.common.join("objects")).unwrap();
            }
            _ => {
                let old = f._dir.path().join("previous-checkout");
                std::fs::rename(&f.root, &old).unwrap();
                let other = f._dir.path().join("unapproved-checkout");
                std::fs::create_dir(&other).unwrap();
                std::os::unix::fs::symlink(&other, &f.root).unwrap();
                f.root = old;
            }
        }
        closed(&mut socket).await;
        tokio::time::sleep(Duration::from_millis(250)).await;
        let before = std::fs::read(f.root.join("child-ticks")).unwrap();
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert_eq!(
            before,
            std::fs::read(f.root.join("child-ticks")).unwrap(),
            "Silent child outlived metadata revocation"
        );
    }
}

#[cfg(target_os = "linux")]
const LINGER_PEER: &str = r#"
import json, os, subprocess, sys, time
common, mode = sys.argv[1:]
child = None
for raw in sys.stdin:
    request = json.loads(raw)
    if request.get('method') == 'session/new':
        print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':{'sessionId':'metadata-lingering'}}),flush=True)
    elif request.get('method') == 'session/prompt':
        child = subprocess.Popen([sys.executable, '-u', '-c', '''import os,sys,time
with open(sys.argv[1], 'ab', buffering=0) as held:
    with open('held-fd-ready','w') as marker: marker.write('ready')
    while True:
        held.write(b'metadata-tick\\n')
        time.sleep(.05)
''', os.path.join(common, 'linger-held-fd')])
        with open('prompt-started','w') as marker: marker.write('started')
        if mode == 'blocked':
            while not os.path.exists('release-permission'): time.sleep(.01)
            print(json.dumps({'jsonrpc':'2.0','id':'x'*(768*1024),'method':'session/request_permission','params':{'sessionId':'metadata-lingering'}}),flush=True)
            with open('permission-emitted','w') as marker: marker.write('sent')
            # Prove that the kernel pipe is full without consuming any byte.
            # Only this causal marker authorizes the test to revoke the grant.
            import array, fcntl, termios
            capacity = fcntl.fcntl(sys.stdin.fileno(), fcntl.F_GETPIPE_SZ)
            while True:
                pending = array.array('i', [0])
                fcntl.ioctl(sys.stdin.fileno(), termios.FIONREAD, pending, True)
                if pending[0] == capacity:
                    with open('stdin-pipe-full','w') as marker: marker.write(str(capacity))
                    break
                time.sleep(.001)
            # Never consume the cancellation response.
            while True: time.sleep(1)
        # Silent mode intentionally leaves the prompt pending until killed.
"#;

#[cfg(target_os = "linux")]
async fn fixture_until(check: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Actual lingering metadata fixture deadline");
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires dedicated real bubblewrap AppArmor/seccomp runtime runner"]
async fn production_browser_acceptance_runtime_metadata_lingering() {
    for mode in ["silent", "blocked"] {
        let mut f = Fixture::new();
        std::fs::write(f.root.join("peer.py"), LINGER_PEER).unwrap();
        std::sync::Arc::get_mut(&mut f.state.config)
            .unwrap()
            .harness = format!(
            "/usr/bin/python3 -u {} {} {mode}",
            f.root.join("peer.py").display(),
            f.common.display(),
        );
        f.grant(&[MetadataPermission::Read, MetadataPermission::Write], None);
        let (mut socket, _server) = connect(&f).await;
        socket
            .send(ClientMessage::Text(
                json!({"jsonrpc":"2.0","id":1,
            "method":"session/new","params":{"cwd":f.root,"mcpServers":[]}})
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let session = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let frame = socket
                    .next()
                    .await
                    .expect("ACP session response")
                    .expect("ACP frame");
                if let ClientMessage::Text(text) = frame {
                    let response: Value = serde_json::from_str(&text).unwrap();
                    if response["id"] == 1 {
                        break response;
                    }
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(session["result"]["sessionId"], "metadata-lingering");
        socket
            .send(ClientMessage::Text(
                json!({"jsonrpc":"2.0","id":2,
            "method":"session/prompt","params":{"sessionId":"metadata-lingering",
            "prompt":[{"type":"text","text":"keep metadata FD open"}]}})
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let held = f.common.join("linger-held-fd");
        fixture_until(|| {
            f.root.join("held-fd-ready").is_file()
                && std::fs::metadata(&held).is_ok_and(|m| m.len() >= 2)
        })
        .await;
        socket.close(None).await.unwrap();
        // Prove this is the actual disconnected-active-prompt linger path,
        // rather than ordinary bridge teardown with no active session.
        fixture_until(|| {
            f.state
                .leases
                .lock()
                .unwrap()
                .get("metadata-lingering")
                .is_some_and(|lease| {
                    lease.expires_at
                        > Instant::now() + Duration::from_secs(f.state.config.linger_secs)
                })
        })
        .await;
        if mode == "blocked" {
            std::fs::write(f.root.join("release-permission"), "release").unwrap();
            fixture_until(|| f.root.join("permission-emitted").is_file()).await;
            fixture_until(|| f.root.join("stdin-pipe-full").is_file()).await;
        }
        {
            let store = f.state.store.lock().unwrap();
            store
                .revoke_local_metadata_grant(&f.project.id, &f.user.id, 1)
                .unwrap();
            store
                .set_local_metadata_grant(
                    &f.project.id,
                    &f.user.id,
                    1,
                    1,
                    &[MetadataPermission::Read, MetadataPermission::Write],
                    true,
                    Some(2),
                    true,
                )
                .unwrap();
        }
        fixture_until(|| {
            !f.state
                .leases
                .lock()
                .unwrap()
                .contains_key("metadata-lingering")
        })
        .await;
        tokio::time::sleep(Duration::from_millis(250)).await;
        let before = std::fs::read(&held).unwrap();
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert_eq!(
            before,
            std::fs::read(&held).unwrap(),
            "A lingering child retained a live writable metadata FD after {mode} revocation/ABA"
        );
    }
}

#[cfg(target_os = "macos")]
#[path = "metadata_macos.rs"]
mod macos_kernel;

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "metadata_kernel_probe.rs"]
mod kernel_probe_code;

#[cfg(target_os = "linux")]
#[path = "metadata_profile_export.rs"]
mod profile_export;
