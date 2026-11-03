use super::{MAX_RESPONSE_BYTES, WorkspaceBackend, history, validate_envelope};
use crate::remote::{RemoteHost, RemoteWorkspace};
use dunlin_domain::RemoteProject;
use dunlin_workspace::git::history::{
    Commit, CommitDetails, CommitFile, CommitPatch, HistoryResponse,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

fn server(response: String) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let task = thread::spawn(move || {
        let start = Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && start.elapsed() < Duration::from_secs(5) =>
                {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("mock relay did not receive a request: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0; 1024];
            let n = stream.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..n]);
            if bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                break;
            }
            assert!(bytes.len() < 16 * 1024);
        }
        let _ = stream.write_all(response.as_bytes());
        String::from_utf8(bytes).unwrap()
    });
    (address, task)
}
fn response(value: impl serde::Serialize) -> String {
    let body = serde_json::to_string(&value).unwrap();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}
fn backend(url: String) -> WorkspaceBackend {
    WorkspaceBackend::Remote(Box::new(RemoteWorkspace {
        host: RemoteHost {
            id: "host".into(),
            name: "Mock".into(),
            url,
            user_name: "member".into(),
            user_role: "member".into(),
            device_id: "device".into(),
        },
        project: RemoteProject {
            host_id: "host".into(),
            project_id: "server-project".into(),
            path: "/not-a-local-project".into(),
        },
        token: "test-history-token".into(),
    }))
}
fn commit() -> Commit {
    Commit {
        oid: "a".repeat(40),
        parents: Vec::new(),
        author: "Author".into(),
        timestamp: 1,
        subject: "root".into(),
        refs: "HEAD -> main".into(),
    }
}

fn inventory() -> dunlin_workspace::git::overview::Overview {
    use dunlin_workspace::git::overview::*;
    Overview {
        worktrees: vec![Worktree {
            path: "/server/never-inspect-locally".into(),
            head: Some("a".repeat(40)),
            branch: Some("refs/heads/main".into()),
            current: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn inventory_transport_is_scoped_authenticated_and_never_opens_server_paths() {
    let scope = "b".repeat(64);
    let (url, task) = server(response(HistoryResponse {
        version: 1,
        scope: scope.clone(),
        value: inventory(),
    }));
    let result = backend(url).git_overview(Some(&scope)).unwrap();
    assert_eq!(result.value, inventory());
    let request = task.join().unwrap();
    assert!(request.starts_with(&format!(
        "GET /v1/git/overview?project=server-project&scope={scope} "
    )));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer test-history-token")
    );
    assert!(!request.contains("/server/"));
    let (url, task) = server(response(HistoryResponse {
        version: 1,
        scope: "c".repeat(64),
        value: inventory(),
    }));
    assert!(
        backend(url)
            .git_overview(Some(&scope))
            .unwrap_err()
            .contains("scope changed")
    );
    task.join().unwrap();
}

#[test]
fn malformed_inventory_and_old_inventory_endpoint_fail_without_fallback() {
    use dunlin_workspace::git::overview::Conflict;
    let scope = "b".repeat(64);
    for kind in ["path", "stages", "current", "oversized"] {
        let mut value = inventory();
        match kind {
            "path" => value.conflicts.push(Conflict {
                path: "../escape".into(),
                stages: vec![1, 2],
            }),
            "stages" => value.conflicts.push(Conflict {
                path: "file".into(),
                stages: vec![3, 2],
            }),
            "current" => value.worktrees[0].current = false,
            _ => value.worktrees[0].locked = Some("x".repeat(2 * 1024 * 1024)),
        }
        let (url, task) = server(response(HistoryResponse {
            version: 1,
            scope: scope.clone(),
            value,
        }));
        assert!(backend(url).git_overview(None).is_err());
        task.join().unwrap();
    }
    let (url, task) =
        server("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into());
    assert!(
        backend(url)
            .git_overview(None)
            .unwrap_err()
            .contains("No local or legacy fallback")
    );
    assert!(task.join().unwrap().starts_with("GET /v1/git/overview?"));
}

#[test]
fn history_transport_encodes_query_uses_auth_and_preserves_the_server_scope() {
    let scope = "b".repeat(64);
    let (url, task) = server(response(HistoryResponse {
        version: history::PROTOCOL_VERSION,
        scope: scope.clone(),
        value: vec![commit()],
    }));
    let value = backend(url).history_load("feature/a&b", 500, None).unwrap();
    assert_eq!(value.scope, scope);
    assert_eq!(value.value, vec![commit()]);
    let request = task.join().unwrap();
    assert!(request.starts_with(
        "GET /v1/git/history?project=server-project&revision=feature/a%26b&limit=500 "
    ));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer test-history-token")
    );
    assert!(!request.contains("/not-a-local-project"));
}

#[test]
fn commit_and_patch_responses_must_match_commit_parent_scope_and_exact_selected_file() {
    let scope = "b".repeat(64);
    let file = CommitFile {
        status: "A".into(),
        path: "file with spaces".into(),
        old_path: None,
    };
    let detail = CommitDetails {
        commit: commit(),
        message: "root".into(),
        parent: None,
        files: vec![file.clone()],
    };
    let (url, task) = server(response(HistoryResponse {
        version: 1,
        scope: scope.clone(),
        value: detail.clone(),
    }));
    assert_eq!(
        backend(url)
            .history_details(&scope, &commit().oid, 0)
            .unwrap(),
        detail
    );
    assert!(
        task.join()
            .unwrap()
            .starts_with("GET /v1/git/history/commit?")
    );
    for mismatch in [false, true] {
        let patch = CommitPatch {
            oid: commit().oid,
            parent: 0,
            file: if mismatch {
                CommitFile {
                    path: "other".into(),
                    ..file.clone()
                }
            } else {
                file.clone()
            },
            patch: "diff metadata".into(),
        };
        let (url, task) = server(response(HistoryResponse {
            version: 1,
            scope: scope.clone(),
            value: patch,
        }));
        let result = backend(url).history_patch(&scope, &commit().oid, 0, 0, &file);
        assert_eq!(result.is_err(), mismatch);
        let request = task.join().unwrap();
        assert!(request.contains("&parent=0&file=0 "));
        assert!(!request.contains("file%20with"));
    }
}

#[test]
fn old_relays_do_not_trigger_local_or_legacy_fallback_and_errors_do_not_echo_server_bodies() {
    let body = "test-history-token must not be displayed";
    let (url, task) = server(format!(
        "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ));
    let error = backend(url).history_load("", 500, None).unwrap_err();
    assert!(error.contains("updated Dunlin relay"));
    assert!(error.contains("No local or legacy fallback"));
    assert!(!error.contains("test-history-token"));
    assert!(task.join().unwrap().starts_with("GET /v1/git/history?"));
}

#[test]
fn redirects_are_never_followed_and_response_sizes_and_protocols_are_bounded() {
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let (url, task) = server(format!(
        "HTTP/1.1 302 Found\r\nLocation: http://{}/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        target.local_addr().unwrap()
    ));
    assert!(
        backend(url)
            .history_load("", 500, None)
            .unwrap_err()
            .contains("redirect")
    );
    task.join().unwrap();
    assert!(
        matches!(target.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    let (url, task) = server(format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        MAX_RESPONSE_BYTES + 1
    ));
    assert!(
        backend(url)
            .history_load("", 500, None)
            .unwrap_err()
            .contains("size limit")
    );
    task.join().unwrap();
    let envelope = HistoryResponse {
        version: 99,
        scope: "b".repeat(64),
        value: (),
    };
    assert!(validate_envelope(&envelope, None).is_err());
    let envelope = HistoryResponse {
        version: 1,
        ..envelope
    };
    assert!(validate_envelope(&envelope, Some(&"c".repeat(64))).is_err());
}
