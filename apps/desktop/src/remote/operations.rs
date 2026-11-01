//! Explicit operation and blame transport; no automatic retry or local fallback.
use super::{RemoteWorkspace, WorkspaceBackend, encode, entry_relative};
use dunlin_workspace::git::{
    blame::{self, Blame},
    history::{self, HistoryResponse},
    operations::{self, Action, Outcome, Plan},
};
use serde::{Deserialize, de::DeserializeOwned};
use std::{io::Read, time::Duration};
#[derive(Clone, Deserialize)]
pub(crate) struct Prepared {
    pub id: String,
    pub plan: Plan,
}
fn request<T: DeserializeOwned>(
    remote: &RemoteWorkspace,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<T, String> {
    super::validate_relay_url(&remote.host.url)?;
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .redirects(0)
        .build();
    let url = format!("{}{path}", remote.host.url.trim_end_matches('/'));
    let req = if body.is_some() {
        agent.post(&url)
    } else {
        agent.get(&url)
    }
    .set("Authorization", &format!("Bearer {}", remote.token))
    .timeout(Duration::from_secs(90));
    let response=match body {Some(body)=>req.send_json(body),None=>req.call()}.map_err(|e| match e {ureq::Error::Status(404|405,_)=>"This Git feature requires an updated relay. No fallback was attempted.".into(),ureq::Error::Status(401|403,_)=>"Relay denied this Git action; check pairing and the project grant.".into(),ureq::Error::Status(409,_)=>"Git state/confirmation changed or execution stopped. Refresh state and prepare again; do not retry the old request.".into(),ureq::Error::Status(400,_)=>"Relay Git preflight refused this action. Check cleanliness, operation state, target and safety limits.".into(),_=>"Relay Git request failed or timed out. A submitted mutation may have run; refresh state. Never automatically retry.".to_string()})?;
    if response.status() != 200 {
        return Err("Unexpected relay Git response/redirect".into());
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read relay Git response")?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Relay Git response exceeds the limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid relay Git response".into())
}
impl WorkspaceBackend {
    pub(crate) fn prepare_git_operation(&self, action: Action) -> Result<Prepared, String> {
        match self {
            Self::Local { root } => Ok(Prepared {
                id: uuid::Uuid::now_v7().to_string(),
                plan: operations::preflight(root, action)?,
            }),
            Self::Remote(remote) => {
                let prepared: Prepared = request(
                    remote,
                    "/v1/git/operation/prepare",
                    Some(serde_json::json!({"project":remote.project.project_id,"action":action})),
                )?;
                if prepared.id.is_empty()
                    || prepared.id.len() > 64
                    || prepared.plan.action != action
                    || prepared.plan.fingerprint.len() > 160
                {
                    return Err("Invalid relay operation plan".into());
                }
                dunlin_workspace::git::overview::validate(&prepared.plan.overview)?;
                Ok(prepared)
            }
        }
    }
    pub(crate) fn execute_git_operation(&self, prepared: &Prepared) -> Result<Outcome, String> {
        match self {
            Self::Local { root } => operations::execute(root, &prepared.plan),
            Self::Remote(remote) => {
                let value: Outcome = request(
                    remote,
                    "/v1/git/operation/execute",
                    Some(serde_json::json!({"project":remote.project.project_id,"id":prepared.id})),
                )?;
                dunlin_workspace::git::overview::validate(&value.overview)?;
                Ok(value)
            }
        }
    }
    pub(crate) fn blame(
        &self,
        path: &str,
        scope: Option<&str>,
        head: Option<&str>,
    ) -> Result<HistoryResponse<Blame>, String> {
        match self {
            Self::Local { root } => {
                let file = entry_relative(root, path)?;
                let root = history::repository_root(root, scope)?;
                let value = blame::load(&root, &file, head)?;
                Ok(HistoryResponse {
                    version: history::PROTOCOL_VERSION,
                    scope: root,
                    value,
                })
            }
            Self::Remote(remote) => {
                let file = entry_relative(&remote.project.path, path)?;
                let mut endpoint = format!(
                    "/v1/git/blame?project={}&path={}",
                    encode(&remote.project.project_id),
                    encode(&file)
                );
                if let Some(scope) = scope {
                    endpoint.push_str(&format!("&scope={}", encode(scope)));
                }
                if let Some(head) = head {
                    endpoint.push_str(&format!("&head={}", encode(head)));
                }
                let value: HistoryResponse<Blame> = request(remote, &endpoint, None)?;
                if value.version != history::PROTOCOL_VERSION
                    || value.scope.len() != 64
                    || !value.scope.bytes().all(|b| b.is_ascii_hexdigit())
                    || scope.is_some_and(|s| s != value.scope)
                    || value.value.path != file
                    || head.is_some_and(|h| h != value.value.head)
                {
                    return Err("Invalid relay blame scope/file/HEAD".into());
                }
                blame::validate(&value.value)?;
                Ok(value)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Prepared, request};
    use crate::remote::{RemoteHost, RemoteWorkspace, WorkspaceBackend};
    use dunlin_workspace::git::operations::{self, Action, Outcome};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        process::Command,
        thread,
        time::{Duration, Instant},
    };
    fn remote(url: String, root: String) -> WorkspaceBackend {
        WorkspaceBackend::Remote(RemoteWorkspace {
            host: RemoteHost {
                id: "host".into(),
                name: "Mock".into(),
                url,
                user_name: "user".into(),
                device_id: "device".into(),
            },
            project: dunlin_domain::RemoteProject {
                host_id: "host".into(),
                project_id: "server-project".into(),
                path: root,
            },
            token: "git-test-token".into(),
        })
    }
    fn server(
        replies: Vec<(u16, String, Option<String>)>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let task = thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body, location) in replies {
                let start = Instant::now();
                let mut stream = loop {
                    match listener.accept() {
                        Ok((s, _)) => break s,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && start.elapsed() < Duration::from_secs(5) =>
                        {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(e) => panic!("mock relay accept: {e}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 4096];
                    let n = stream.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 256 * 1024);
                    if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                        let len = header
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .map_or(0, |s| s.trim().parse::<usize>().unwrap());
                        if bytes.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(bytes).unwrap());
                let location = location.map_or(String::new(), |l| format!("Location: {l}\r\n"));
                let _=stream.write_all(format!("HTTP/1.1 {status} Reply\r\nContent-Type: application/json\r\n{location}Connection: close\r\nContent-Length: {}\r\n\r\n{body}",body.len()).as_bytes());
            }
            requests
        });
        (url, task)
    }
    #[test]
    fn remote_mutations_send_only_the_server_token_never_execute_local_plan_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let git = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .args(["-C", root])
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        };
        git(&["init", "--template=", "-qb", "main"]);
        std::fs::create_dir(dir.path().join(".git/hooks")).unwrap();
        git(&[
            "config",
            "core.hooksPath",
            dir.path().join(".git/hooks").to_str().unwrap(),
        ]);
        git(&["config", "user.name", "Test"]);
        git(&["config", "core.fsmonitor", "false"]);
        git(&["config", "core.attributesFile", "/dev/null"]);
        git(&["config", "user.email", "test@example.invalid"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.path().join("file"), "base").unwrap();
        git(&["add", "file"]);
        git(&["commit", "-qm", "base"]);
        std::fs::write(dir.path().join("file"), "local keep").unwrap();
        let action = Action::StashPush {
            message: "remote".into(),
            include_untracked: true,
        };
        let plan = operations::preflight(root, action.clone()).unwrap();
        let outcome = Outcome {
            success: true,
            message: "server result".into(),
            overview: plan.overview.clone(),
        };
        let body = serde_json::json!({"id":"server-one-use-token","plan":plan}).to_string();
        let (url, task) = server(vec![
            (200, body, None),
            (200, serde_json::to_string(&outcome).unwrap(), None),
        ]);
        let backend = remote(url, root.into());
        let prepared = backend.prepare_git_operation(action).unwrap();
        backend.execute_git_operation(&prepared).unwrap();
        let requests = task.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|r| {
            r.to_ascii_lowercase()
                .contains("authorization: bearer git-test-token")
        }));
        assert!(requests[0].starts_with("POST /v1/git/operation/prepare "));
        assert!(requests[1].starts_with("POST /v1/git/operation/execute "));
        let body: serde_json::Value =
            serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"project":"server-project","id":"server-one-use-token"})
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file")).unwrap(),
            "local keep"
        );
    }
    #[test]
    fn mutation_transport_never_follows_redirects_retries_old_relays_or_echoes_server_secrets() {
        for status in [302, 404, 409, 401] {
            let (url, task) = server(vec![(
                status,
                "{\"error\":\"private-server-secret\"}".into(),
                Some("http://127.0.0.1:9/never".into()),
            )]);
            let WorkspaceBackend::Remote(remote) = remote(url, "/server/never-local".into()) else {
                panic!()
            };
            let result: Result<Prepared, _> = request(
                &remote,
                "/v1/git/operation/prepare",
                Some(
                    serde_json::json!({"project":"server-project","action":{"kind":"worktree_prune"}}),
                ),
            );
            let error = result.err().expect("HTTP failure");
            assert!(!error.contains("private-server-secret"));
            assert!(!error.contains("git-test-token"));
            assert_eq!(task.join().unwrap().len(), 1);
        }
    }
}
