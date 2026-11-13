use std::time::Duration;

use axum::http::StatusCode;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

use super::{Fixture, router};

fn endpoint(
    f: &Fixture,
    addr: std::net::SocketAddr,
    token: Option<&str>,
) -> tokio_tungstenite::tungstenite::http::Request<()> {
    let url = format!(
        "ws://{addr}/v1/terminal?project={}&rows=10&cols=40",
        f.project
    );
    let mut request = url.into_client_request().unwrap();
    if let Some(token) = token {
        request.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
    }
    request
}

fn status(error: tokio_tungstenite::tungstenite::Error) -> StatusCode {
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => response.status(),
        other => panic!("expected an HTTP handshake rejection, got {other}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_terminal_requires_auth_and_a_project_grant_then_streams_pty_bytes() {
    let f = Fixture::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(f.state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let denied = tokio_tungstenite::connect_async(endpoint(&f, addr, None))
        .await
        .unwrap_err();
    assert_eq!(status(denied), StatusCode::UNAUTHORIZED);
    let denied = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.ungranted_token)))
        .await
        .unwrap_err();
    assert_eq!(status(denied), StatusCode::FORBIDDEN);

    let (mut socket, _) = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.token)))
        .await
        .unwrap();
    let ready = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ready = ready.into_text().unwrap();
    let ready: serde_json::Value = serde_json::from_str(&ready).unwrap();
    assert_eq!(ready["type"], "ready");
    assert_eq!(ready["project"], f.project);

    // The relay shell starts with shell integration, so wait for command output
    // rather than requiring the very first frame to contain it.
    socket
        .send(Message::Binary(
            b"printf 'dunlin-relay-terminal\\n'; exit\n"
                .as_slice()
                .into(),
        ))
        .await
        .unwrap();
    let mut output = Vec::new();
    let mut exited = false;
    for _ in 0..80 {
        let Some(frame) = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .ok()
            .flatten()
        else {
            break;
        };
        match frame.unwrap() {
            Message::Binary(bytes) => output.extend_from_slice(&bytes),
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "exit" {
                    exited = true;
                    break;
                }
                if value["type"] == "error" {
                    panic!("relay terminal error: {value}");
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
        if String::from_utf8_lossy(&output).contains("dunlin-relay-terminal") {
            break;
        }
    }
    assert!(
        String::from_utf8_lossy(&output).contains("dunlin-relay-terminal"),
        "missing PTY output: {output:?}"
    );
    // Exit may legitimately race the last output frame; drain until it arrives.
    for _ in 0..200 {
        if exited {
            break;
        }
        let Some(frame) = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .ok()
            .flatten()
        else {
            break;
        };
        match frame.unwrap() {
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "exit" {
                    exited = true;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    assert!(exited, "relay did not report the shell exit");
}

type TerminalSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
struct TerminalServer(tokio::task::JoinHandle<()>);
impl Drop for TerminalServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn start_terminal_server(f: &Fixture) -> (std::net::SocketAddr, TerminalServer) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(f.state.clone());
    let job = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, TerminalServer(job))
}
async fn ready_pid(socket: &mut TerminalSocket) -> i32 {
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(frame) = socket.next().await {
            if let Message::Text(text) = frame.unwrap() {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "ready" {
                    return value["pid"].as_i64().unwrap() as i32;
                }
            }
        }
        panic!("Terminal closed before ready");
    })
    .await
    .unwrap()
}
async fn expect_terminal_output(socket: &mut TerminalSocket, marker: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut output = Vec::new();
        while let Some(frame) = socket.next().await {
            match frame.unwrap() {
                Message::Binary(bytes) => {
                    output.extend_from_slice(&bytes);
                    if String::from_utf8_lossy(&output).contains(marker) {
                        return;
                    }
                }
                Message::Close(_) => panic!("Terminal closed before output"),
                _ => {}
            }
        }
        panic!("Terminal closed without expected marker");
    })
    .await
    .unwrap();
}
async fn expect_revoked_terminal(socket: &mut TerminalSocket) {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut output = Vec::new();
        while let Some(frame) = socket.next().await {
            match frame.unwrap() {
                Message::Binary(bytes) => {
                    output.extend_from_slice(&bytes);
                    assert!(!String::from_utf8_lossy(&output).contains("STALE_"));
                }
                Message::Text(text) => {
                    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                    assert_ne!(value["type"], "exit");
                }
                Message::Close(frame) => {
                    assert_eq!(
                        u16::from(frame.expect("Revocation requires policy close").code),
                        1008
                    );
                    return;
                }
                _ => {}
            }
        }
        panic!("Missing terminal revocation close");
    })
    .await
    .unwrap();
}
fn terminal_pid_alive(pid: i32) -> bool {
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        && stat.split_whitespace().nth(2) == Some("Z")
    {
        return false;
    }
    unsafe { libc::kill(pid, 0) == 0 }
}
async fn expect_terminal_pid_dead(pid: i32) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while terminal_pid_alive(pid) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "PTY process {pid} survived revocation"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_native_grant_aba_kills_hup_ignoring_shell_and_foreground_job_without_forwarding_bytes()
 {
    let f = Fixture::new();
    std::fs::write(
        f.root.join("pty_epoch_peer.py"),
        r#"
import os, select, signal, sys
signal.signal(signal.SIGHUP, signal.SIG_IGN)
with open('foreground-pid','w') as f: f.write(str(os.getpid()))
print('FOREGROUND_READY',flush=True)
emitted=False
while True:
    if not emitted and os.path.exists('release-stale-output'):
        print('STALE_OUTPUT_MARKER',flush=True)
        with open('stale-output-emitted','w') as f: f.write('emitted')
        emitted=True
    ready,_,_=select.select([sys.stdin.fileno()],[],[],.01)
    if ready:
        data=os.read(sys.stdin.fileno(),65536)
        if not data: break
        with open('foreground-input','ab') as f: f.write(data)
"#,
    )
    .unwrap();
    let (addr, _server) = start_terminal_server(&f).await;
    let (mut socket, _) = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.token)))
        .await
        .unwrap();
    let shell = ready_pid(&mut socket).await;
    socket
        .send(Message::Binary(
            b"trap '' HUP; python3 -u pty_epoch_peer.py\n"
                .as_slice()
                .into(),
        ))
        .await
        .unwrap();
    expect_terminal_output(&mut socket, "FOREGROUND_READY").await;
    let foreground: i32 = std::fs::read_to_string(f.root.join("foreground-pid"))
        .unwrap()
        .parse()
        .unwrap();
    assert_ne!(shell, foreground);
    assert!(terminal_pid_alive(shell));
    assert!(terminal_pid_alive(foreground));
    {
        let store = f.state.store.lock().unwrap();
        let (user, _) = store.authenticate(&f.token).unwrap().unwrap();
        // Queue real PTY output while its relay authorization check is blocked.
        std::fs::write(f.root.join("release-stale-output"), "release").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !f.root.join("stale-output-emitted").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "PTY peer did not emit"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        store.revoke_grant(&f.project, &user.id).unwrap();
        store.grant(&f.project, &user.id).unwrap();
        assert!(
            store
                .project_for_user(&f.project, &user.id)
                .unwrap()
                .is_some()
        );
    }
    let _ = socket
        .send(Message::Binary(b"STALE_INPUT_MARKER\n".as_slice().into()))
        .await;
    expect_revoked_terminal(&mut socket).await;
    expect_terminal_pid_dead(shell).await;
    expect_terminal_pid_dead(foreground).await;
    assert!(!f.root.join("foreground-input").exists());
    let (mut fresh, _) = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.token)))
        .await
        .unwrap();
    let fresh_pid = ready_pid(&mut fresh).await;
    assert_ne!(fresh_pid, shell);
    fresh
        .send(Message::Binary(
            b"printf 'FRESH_'; printf 'PTY_OK\\n'\n".as_slice().into(),
        ))
        .await
        .unwrap();
    expect_terminal_output(&mut fresh, "FRESH_PTY_OK").await;
    fresh
        .send(Message::Text(r#"{"type":"close"}"#.into()))
        .await
        .unwrap();
    expect_terminal_pid_dead(fresh_pid).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_device_revocation_closes_1008_and_kills_the_existing_shell() {
    let f = Fixture::new();
    let (addr, _server) = start_terminal_server(&f).await;
    let (mut socket, _) = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.token)))
        .await
        .unwrap();
    let pid = ready_pid(&mut socket).await;
    {
        let store = f.state.store.lock().unwrap();
        let (_, device) = store.authenticate(&f.token).unwrap().unwrap();
        store.revoke_device(&device.id).unwrap();
    }
    expect_revoked_terminal(&mut socket).await;
    expect_terminal_pid_dead(pid).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_revocation_watchdog_still_kills_a_pty_with_a_blocked_input_writer() {
    let f = Fixture::new();
    std::fs::write(
        f.root.join("blocked_pty.py"),
        r#"
import os, signal, time
signal.signal(signal.SIGHUP, signal.SIG_IGN)
with open('blocked-foreground-pid','w') as f: f.write(str(os.getpid()))
print('BLOCKED_FOREGROUND_READY',flush=True)
time.sleep(60)
"#,
    )
    .unwrap();
    let (addr, _server) = start_terminal_server(&f).await;
    let (mut socket, _) = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.token)))
        .await
        .unwrap();
    let shell = ready_pid(&mut socket).await;
    socket
        .send(Message::Binary(
            b"trap '' HUP; stty raw -echo; python3 -u blocked_pty.py\n"
                .as_slice()
                .into(),
        ))
        .await
        .unwrap();
    expect_terminal_output(&mut socket, "BLOCKED_FOREGROUND_READY").await;
    let child: i32 = std::fs::read_to_string(f.root.join("blocked-foreground-pid"))
        .unwrap()
        .parse()
        .unwrap();
    // A raw terminal whose foreground process never reads fills its input queue.
    // The watchdog must remain runnable while the blocking write is in flight.
    socket
        .send(Message::Binary(vec![b'x'; 2 * 1024 * 1024].into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    {
        let store = f.state.store.lock().unwrap();
        let (user, _) = store.authenticate(&f.token).unwrap().unwrap();
        store.revoke_grant(&f.project, &user.id).unwrap();
        store.grant(&f.project, &user.id).unwrap();
    }
    expect_revoked_terminal(&mut socket).await;
    expect_terminal_pid_dead(shell).await;
    expect_terminal_pid_dead(child).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_blocked_input_then_client_disconnect_releases_processes_and_permit_without_revocation()
 {
    let f = Fixture::new();
    let initial_permits = f.state.terminal_sessions.available_permits();
    std::fs::write(
        f.root.join("disconnected_pty.py"),
        r#"
import os, signal, time
signal.signal(signal.SIGHUP, signal.SIG_IGN)
with open('disconnected-foreground-pid','w') as f: f.write(str(os.getpid()))
print('DISCONNECTED_FOREGROUND_READY',flush=True)
time.sleep(60)
"#,
    )
    .unwrap();
    let (addr, _server) = start_terminal_server(&f).await;
    let (mut socket, _) = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.token)))
        .await
        .unwrap();
    let shell = ready_pid(&mut socket).await;
    assert_eq!(
        f.state.terminal_sessions.available_permits(),
        initial_permits - 1
    );
    socket
        .send(Message::Binary(
            b"trap '' HUP; stty raw -echo; python3 -u disconnected_pty.py\n"
                .as_slice()
                .into(),
        ))
        .await
        .unwrap();
    expect_terminal_output(&mut socket, "DISCONNECTED_FOREGROUND_READY").await;
    let child: i32 = std::fs::read_to_string(f.root.join("disconnected-foreground-pid"))
        .unwrap()
        .parse()
        .unwrap();
    socket
        .send(Message::Binary(vec![b'x'; 2 * 1024 * 1024].into()))
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let _ = tokio::time::timeout_at(deadline, socket.close(None))
        .await
        .expect("Client close did not finish within the terminal lifetime budget");
    drop(socket);
    while terminal_pid_alive(shell)
        || terminal_pid_alive(child)
        || f.state.terminal_sessions.available_permits() != initial_permits
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "Disconnected blocked PTY retained its process or permit"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let store = f.state.store.lock().unwrap();
    let (user, _) = store.authenticate(&f.token).unwrap().unwrap();
    assert!(store.capture_native_grant(&f.project, &user.id).is_ok());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_terminal_drains_sizable_final_output_before_reporting_a_fast_shell_exit() {
    let f = Fixture::new();
    let (addr, _server) = start_terminal_server(&f).await;
    for _ in 0..3 {
        let (mut socket, _) = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.token)))
            .await
            .unwrap();
        let pid = ready_pid(&mut socket).await;
        // Numeric character construction keeps the expected output marker and
        // long payload out of the command echo; only real process output counts.
        let command = b"python3 -c 'import sys;sys.stdout.write(chr(90)*131072+chr(10)+chr(69)+chr(78)+chr(68)+chr(95)+chr(70)+chr(73)+chr(78)+chr(65)+chr(76)+chr(10));sys.stdout.flush()'; exit\n";
        socket
            .send(Message::Binary(command.as_slice().into()))
            .await
            .unwrap();
        let output = tokio::time::timeout(Duration::from_secs(10), async {
            let mut output = Vec::new();
            while let Some(frame) = socket.next().await {
                match frame.unwrap() {
                    Message::Binary(bytes) => output.extend_from_slice(&bytes),
                    Message::Text(text) => {
                        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                        assert_ne!(value["type"], "error", "Terminal failure: {value}");
                        if value["type"] == "exit" {
                            return output;
                        }
                    }
                    Message::Close(frame) => {
                        panic!("Terminal closed before normal exit: {frame:?}")
                    }
                    _ => {}
                }
            }
            panic!("No terminal exit message");
        })
        .await
        .unwrap();
        let output = String::from_utf8_lossy(&output);
        assert!(
            output.contains(&"Z".repeat(131072)),
            "Final process payload was truncated"
        );
        assert!(
            output.contains("END_FINAL"),
            "Last output marker missing before exit"
        );
        expect_terminal_pid_dead(pid).await;
    }
}

struct MetadataCheckout {
    _dir: tempfile::TempDir,
    common: std::path::PathBuf,
    user: String,
}
fn metadata_checkout(f: &Fixture) -> MetadataCheckout {
    let dir = tempfile::tempdir().unwrap();
    let common = dir.path().join("repo.git");
    let git = common.join("worktrees/terminal");
    std::fs::create_dir_all(&git).unwrap();
    std::fs::create_dir_all(common.join("objects")).unwrap();
    std::fs::write(f.root.join(".git"), format!("gitdir: {}\n", git.display())).unwrap();
    std::fs::write(git.join("commondir"), "../..").unwrap();
    std::fs::write(git.join("gitdir"), f.root.join(".git").to_str().unwrap()).unwrap();
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main").unwrap();
    std::fs::write(common.join("HEAD"), "ref: refs/heads/main").unwrap();
    let preparation = f
        .state
        .store
        .lock()
        .unwrap()
        .prepare_local_metadata(&f.project)
        .unwrap();
    let prepared = preparation.capture(&git, &common).unwrap();
    let store = f.state.store.lock().unwrap();
    let (user, _) = store.authenticate(&f.token).unwrap().unwrap();
    let approval = store.commit_local_metadata(prepared, None).unwrap();
    let native = store.capture_native_grant(&f.project, &user.id).unwrap();
    store
        .set_local_metadata_grant(
            &f.project,
            &user.id,
            approval.revision,
            native.revision,
            &[crate::store::local_metadata::MetadataPermission::Read],
            true,
            None,
            false,
        )
        .unwrap();
    MetadataCheckout {
        _dir: dir,
        common,
        user: user.id,
    }
}
fn metadata_write(f: &Fixture, metadata: &MetadataCheckout) {
    let store = f.state.store.lock().unwrap();
    let approval = store.local_metadata(&f.project).unwrap().unwrap();
    let native = store
        .capture_native_grant(&f.project, &metadata.user)
        .unwrap();
    store
        .set_local_metadata_grant(
            &f.project,
            &metadata.user,
            approval.revision,
            native.revision,
            &[
                crate::store::local_metadata::MetadataPermission::Read,
                crate::store::local_metadata::MetadataPermission::Write,
            ],
            true,
            Some(1),
            true,
        )
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_linked_checkout_requires_own_write_grant_and_remains_a_host_shell() {
    let f = Fixture::new();
    let metadata = metadata_checkout(&f);
    let (addr, _server) = start_terminal_server(&f).await;
    let denied = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.token)))
        .await
        .unwrap_err();
    assert_eq!(
        status(denied),
        StatusCode::FORBIDDEN,
        "Read-only metadata cannot authorize an RW shell"
    );
    {
        let store = f.state.store.lock().unwrap();
        let (other, _) = store.authenticate(&f.ungranted_token).unwrap().unwrap();
        store.grant(&f.project, &other.id).unwrap();
    }
    let denied = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.ungranted_token)))
        .await
        .unwrap_err();
    assert_eq!(
        status(denied),
        StatusCode::FORBIDDEN,
        "Another native grantee cannot borrow the owner's metadata grant"
    );
    metadata_write(&f, &metadata);
    let (mut socket, _) = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.token)))
        .await
        .unwrap();
    let pid = ready_pid(&mut socket).await;
    socket
        .send(Message::Binary(
            b"printf 'LINKED_'; printf 'HOST_SHELL_OK\\n'\n"
                .as_slice()
                .into(),
        ))
        .await
        .unwrap();
    expect_terminal_output(&mut socket, "LINKED_HOST_SHELL_OK").await;
    socket
        .send(Message::Text(r#"{"type":"close"}"#.into()))
        .await
        .unwrap();
    expect_terminal_pid_dead(pid).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_metadata_grant_aba_kills_blocked_foreground_and_shell_without_refreshing_authority()
 {
    let f = Fixture::new();
    let metadata = metadata_checkout(&f);
    metadata_write(&f, &metadata);
    std::fs::write(
        f.root.join("metadata_blocked.py"),
        r#"
import os, signal, time
signal.signal(signal.SIGHUP,signal.SIG_IGN)
with open('metadata-foreground-pid','w') as f: f.write(str(os.getpid()))
print('METADATA_BLOCKED_READY',flush=True)
time.sleep(60)
"#,
    )
    .unwrap();
    let (addr, _server) = start_terminal_server(&f).await;
    let (mut socket, _) = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.token)))
        .await
        .unwrap();
    let shell = ready_pid(&mut socket).await;
    socket
        .send(Message::Binary(
            b"trap '' HUP; stty raw -echo; python3 -u metadata_blocked.py\n"
                .as_slice()
                .into(),
        ))
        .await
        .unwrap();
    expect_terminal_output(&mut socket, "METADATA_BLOCKED_READY").await;
    let child = std::fs::read_to_string(f.root.join("metadata-foreground-pid"))
        .unwrap()
        .parse()
        .unwrap();
    socket
        .send(Message::Binary(vec![b'x'; 2 * 1024 * 1024].into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    {
        let store = f.state.store.lock().unwrap();
        store
            .revoke_local_metadata_grant(&f.project, &metadata.user, 2)
            .unwrap();
        store
            .set_local_metadata_grant(
                &f.project,
                &metadata.user,
                1,
                1,
                &[
                    crate::store::local_metadata::MetadataPermission::Read,
                    crate::store::local_metadata::MetadataPermission::Write,
                ],
                true,
                Some(3),
                true,
            )
            .unwrap();
    }
    expect_revoked_terminal(&mut socket).await;
    expect_terminal_pid_dead(shell).await;
    expect_terminal_pid_dead(child).await;
    let (mut fresh, _) = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.token)))
        .await
        .unwrap();
    let fresh_pid = ready_pid(&mut fresh).await;
    fresh
        .send(Message::Text(r#"{"type":"close"}"#.into()))
        .await
        .unwrap();
    expect_terminal_pid_dead(fresh_pid).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_metadata_objects_identity_and_settings_changes_are_seen_without_client_input() {
    for change in ["objects", "settings"] {
        let f = Fixture::new();
        let metadata = metadata_checkout(&f);
        metadata_write(&f, &metadata);
        let (addr, _server) = start_terminal_server(&f).await;
        let (mut socket, _) = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.token)))
            .await
            .unwrap();
        let shell = ready_pid(&mut socket).await;
        match change {
            "objects" => {
                std::fs::rename(
                    metadata.common.join("objects"),
                    metadata.common.join("old-objects"),
                )
                .unwrap();
                std::fs::create_dir(metadata.common.join("objects")).unwrap();
            }
            "settings" => f
                .state
                .store
                .lock()
                .unwrap()
                .save_project_settings(&f.project, "{\"changed\":true}")
                .unwrap(),
            _ => unreachable!(),
        }
        expect_revoked_terminal(&mut socket).await;
        expect_terminal_pid_dead(shell).await;
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_raw_project_alias_retarget_kills_shell_even_when_original_checkout_is_intact() {
    let mut f = Fixture::new();
    let alias_dir = tempfile::tempdir().unwrap();
    let alias = alias_dir.path().join("workspace-alias");
    std::os::unix::fs::symlink(&f.root, &alias).unwrap();
    {
        let store = f.state.store.lock().unwrap();
        let (user, _) = store.authenticate(&f.token).unwrap().unwrap();
        let project = store
            .create_project("terminal-alias", alias.to_str().unwrap())
            .unwrap();
        store.grant(&project.id, &user.id).unwrap();
        f.project = project.id;
    }
    let metadata = metadata_checkout(&f);
    metadata_write(&f, &metadata);
    let (addr, _server) = start_terminal_server(&f).await;
    let (mut socket, _) = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.token)))
        .await
        .unwrap();
    let shell = ready_pid(&mut socket).await;
    let other = alias_dir.path().join("other-checkout");
    std::fs::create_dir(&other).unwrap();
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(other, &alias).unwrap();
    assert!(f.root.join(".git").is_file());
    expect_revoked_terminal(&mut socket).await;
    expect_terminal_pid_dead(shell).await;
}
