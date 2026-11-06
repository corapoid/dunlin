//! One explicit backup file, on the default branch; credentials stay in gh/glab.
use crate::{
    git::hosting::{Provider, RepositoryLink},
    github::process,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::time::Duration;

pub const FILE: &str = "dunlin-backup.json";
// GitHub's contents API embeds base64 only for files up to 1 MiB.
pub const MAX_BYTES: usize = 1024 * 1024;
#[cfg(test)]
thread_local! { static TEST_CLI: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) }; }

fn request(
    link: &RepositoryLink,
    method: &str,
    endpoint: &str,
    body: Option<Value>,
) -> Result<Option<Value>, String> {
    let (mut command, host) = match link.provider {
        Provider::GitHub => (crate::github::cli_command(), "github.com"),
        Provider::GitLab => (crate::gitlab::command(), "gitlab.com"),
    };
    #[cfg(test)]
    if let Some(path) = TEST_CLI.with(|cli| cli.borrow().clone()) {
        let mut mock = std::process::Command::new(path);
        for (key, value) in command.get_envs() {
            if let Some(value) = value {
                mock.env(key, value);
            } else {
                mock.env_remove(key);
            }
        }
        command = mock;
    }
    command.args([
        "api",
        "--hostname",
        host,
        "--method",
        method,
        "--include",
        endpoint,
    ]);
    let input = body.map(|body| body.to_string().into_bytes());
    if input.is_some() {
        command.args(["--input", "-"]);
    }
    let (status, output) = process::run(command, input, Duration::from_secs(60), (MAX_BYTES * 2) as u64)
        .map_err(|_| "Hosting CLI failed or timed out. Check authentication/access; inspect the remote file before retrying an export.".to_string())?;
    let (code, bytes) = response(&output)?;
    if code == 404 && method == "GET" {
        return Ok(None);
    }
    if !status.success() || !(200..300).contains(&code) {
        return Err("Repository backup request failed. Check authentication, repository access, branch protection and concurrent changes. No automatic retry.".into());
    }
    serde_json::from_slice(bytes)
        .map(Some)
        .map_err(|_| "Invalid repository backup response".into())
}

fn response(output: &[u8]) -> Result<(u16, &[u8]), String> {
    let separator = output
        .windows(4)
        .position(|s| s == b"\r\n\r\n")
        .map(|i| (i, 4))
        .or_else(|| output.windows(2).position(|s| s == b"\n\n").map(|i| (i, 2)))
        .ok_or("Missing hosting HTTP status")?;
    let headers =
        std::str::from_utf8(&output[..separator.0]).map_err(|_| "Invalid hosting HTTP headers")?;
    let code = headers
        .lines()
        .next()
        .and_then(|s| s.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or("Invalid hosting HTTP status")?;
    Ok((code, &output[separator.0 + separator.1..]))
}

fn target(url: &str) -> Result<(RepositoryLink, String, String), String> {
    let link = RepositoryLink::parse(url)
        .ok_or("Use a GitHub.com or GitLab.com repository URL (no credentials)")?;
    let name = link.url.splitn(4, '/').nth(3).unwrap();
    let endpoint = match link.provider {
        Provider::GitHub => format!("repos/{name}"),
        Provider::GitLab => format!("projects/{}", name.replace('/', "%2F")),
    };
    let repo = request(&link, "GET", &endpoint, None)?
        .ok_or("Backup repository not found or not accessible")?;
    let identity_key = if link.provider == Provider::GitHub {
        "full_name"
    } else {
        "path_with_namespace"
    };
    if !repo[identity_key]
        .as_str()
        .is_some_and(|identity| identity.eq_ignore_ascii_case(name))
    {
        return Err("Backup repository identity changed; enter its current canonical URL".into());
    }
    let branch = repo["default_branch"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 255 && !s.chars().any(char::is_control))
        .ok_or("Initialize the backup repository with a default branch first")?
        .to_string();
    let file = match link.provider {
        Provider::GitHub => format!("{endpoint}/contents/{FILE}"),
        Provider::GitLab => format!("{endpoint}/repository/files/{FILE}"),
    };
    Ok((link, branch, file))
}

fn encoded(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn file_endpoint(file: &str, branch: &str) -> String {
    format!("{file}?ref={}", encoded(branch))
}

pub fn download(url: &str) -> Result<Vec<u8>, String> {
    let (link, branch, file) = target(url)?;
    let data = request(&link, "GET", &file_endpoint(&file, &branch), None)?
        .ok_or("No dunlin-backup.json on the repository's default branch")?;
    decode(&data)
}
fn decode(data: &Value) -> Result<Vec<u8>, String> {
    if data["encoding"] != "base64"
        || data["size"].as_u64().is_none_or(|s| s > MAX_BYTES as u64)
        || data.get("type").is_some_and(|t| t != "file")
        || data.get("submodule_git_url").is_some()
    {
        return Err("Backup must be a regular base64 file, at most 1 MiB".into());
    }
    let content = data["content"].as_str().ok_or("Missing backup content")?;
    if content.len() > MAX_BYTES * 2 {
        return Err("Backup exceeds 1 MiB".into());
    }
    let bytes = STANDARD
        .decode(
            content
                .bytes()
                .filter(|b| !b.is_ascii_whitespace())
                .collect::<Vec<_>>(),
        )
        .map_err(|_| "Invalid backup encoding")?;
    if bytes.len() > MAX_BYTES {
        return Err("Backup exceeds 1 MiB".into());
    }
    Ok(bytes)
}

pub fn upload(url: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_BYTES {
        return Err("Backup exceeds 1 MiB".into());
    }
    let (link, branch, file) = target(url)?;
    let existing = request(&link, "GET", &file_endpoint(&file, &branch), None)?;
    if let Some(data) = &existing {
        let old = decode(data)?;
        let value: Value =
            serde_json::from_slice(&old).map_err(|_| "Refusing to replace a non-Dunlin file")?;
        if value["format"] != "dunlin-backup" || value["version"] != 1 {
            return Err("Refusing to replace a non-Dunlin or unsupported backup".into());
        }
        if old == bytes {
            return Ok(());
        }
    }
    let mut body = json!({"branch":branch, "content":STANDARD.encode(bytes)});
    let method = match link.provider {
        Provider::GitHub => {
            body["message"] = json!("Update Dunlin backup");
            if let Some(old) = &existing {
                body["sha"] = old
                    .get("sha")
                    .filter(|v| v.is_string())
                    .ok_or("Missing backup revision")?
                    .clone();
            }
            "PUT"
        }
        Provider::GitLab => {
            body["commit_message"] = json!("Update Dunlin backup");
            body["encoding"] = json!("base64");
            if let Some(old) = &existing {
                body["last_commit_id"] = old
                    .get("last_commit_id")
                    .filter(|v| v.is_string())
                    .ok_or("Missing backup revision")?
                    .clone();
                "PUT"
            } else {
                "POST"
            }
        }
    };
    request(&link, method, &file, Some(body))?;
    Ok(())
}

pub fn clone(url: &str, destination: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let link = RepositoryLink::parse(url).ok_or("Invalid repository URL")?;
    let name = link.url.splitn(4, '/').nth(3).unwrap();
    if !destination.is_absolute() {
        return Err("Choose an absolute download folder".into());
    }
    std::fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    let destination = std::fs::canonicalize(destination).map_err(|e| e.to_string())?;
    let host = if link.provider == Provider::GitHub {
        "github.com"
    } else {
        "gitlab.com"
    };
    let root = destination.join(host);
    match std::fs::create_dir(&root) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.to_string()),
    }
    if !std::fs::symlink_metadata(&root).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        return Err("The hosting folder must be a real directory, not a symlink".into());
    }
    match link.provider {
        Provider::GitHub => crate::github::imports::clone_repository(name, &root),
        Provider::GitLab => crate::gitlab::clone_repository(name, &root),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn http_errors_encoding_and_refs_are_bounded() {
        assert_eq!(
            response(b"HTTP/2.0 404 Not Found\r\nX: a\r\n\r\n{}").unwrap(),
            (404, b"{}".as_slice())
        );
        assert_eq!(response(b"HTTP/1.1 200 OK\nX: a\n\n{}").unwrap().0, 200);
        assert!(response(b"{}").is_err());
        assert_eq!(
            file_endpoint("file", "feature/a?x=1"),
            "file?ref=feature%2Fa%3Fx%3D1"
        );
        let mut data = json!({"type":"file", "encoding":"base64", "size":2, "content":"e3\n0="});
        assert_eq!(decode(&data).unwrap(), b"{}");
        data["type"] = json!("symlink");
        assert!(decode(&data).is_err());
        data["type"] = json!("file");
        data["size"] = json!(MAX_BYTES + 1);
        assert!(decode(&data).is_err());
        assert!(upload("https://token@github.com/a/b", b"{}").is_err());
    }
    struct Fixture(tempfile::TempDir);
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let script = dir.path().join("mock-hosting");
            std::fs::write(&script, r#"#!/usr/bin/env python3
import json, sys, pathlib
root = pathlib.Path(__file__).parent
a = sys.argv[1:]
method = a[a.index('--method')+1]
endpoint = a[a.index('--include')+1]
body = json.load(sys.stdin) if '--input' in a else None
with (root/'calls').open('a') as f: f.write(json.dumps({'args':a,'method':method,'endpoint':endpoint,'body':body})+'\n')
responses = json.loads((root/'responses').read_text())
code, data = responses.get(method+' '+endpoint, [403, {'message':'secret must not escape'}])
sys.stdout.write('HTTP/2.0 '+str(code)+' Status\r\nContent-Type: application/json\r\n\r\n'+json.dumps(data))
sys.exit(0 if code < 400 else 1)
"#).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            TEST_CLI.with(|cli| *cli.borrow_mut() = Some(script));
            Self(dir)
        }
        fn responses(&self, data: Value) {
            std::fs::write(self.0.path().join("responses"), data.to_string()).unwrap();
        }
        fn calls(&self) -> Vec<Value> {
            std::fs::read_to_string(self.0.path().join("calls"))
                .unwrap()
                .lines()
                .map(|s| serde_json::from_str(s).unwrap())
                .collect()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            TEST_CLI.with(|cli| *cli.borrow_mut() = None);
        }
    }
    #[test]
    fn github_and_gitlab_create_read_and_update_only_the_named_file_and_revision() {
        let f = Fixture::new();
        let bytes = br#"{"format":"dunlin-backup","version":1,"settings":{"chat_logo":false}}"#;
        let old = json!({"type":"file","size":bytes.len(),"encoding":"base64","content":STANDARD.encode(bytes),"sha":"revision","last_commit_id":"commit"});
        for (host, repo, file) in [
            (
                "github.com",
                "repos/o/r",
                "repos/o/r/contents/dunlin-backup.json",
            ),
            (
                "gitlab.com",
                "projects/g%2Fsub%2Fr",
                "projects/g%2Fsub%2Fr/repository/files/dunlin-backup.json",
            ),
        ] {
            let github = host == "github.com";
            let url = if github {
                "https://github.com/o/r"
            } else {
                "https://gitlab.com/g/sub/r"
            };
            let metadata = if github {
                json!({"full_name":"o/r","default_branch":"backup/main"})
            } else {
                json!({"path_with_namespace":"g/sub/r","default_branch":"backup/main"})
            };
            let endpoint = format!("{file}?ref=backup%2Fmain");
            let mut replies = serde_json::Map::new();
            replies.insert(format!("GET {repo}"), json!([200, metadata]));
            replies.insert(format!("GET {endpoint}"), json!([404, {}]));
            let create = if github { "PUT" } else { "POST" };
            replies.insert(format!("{create} {file}"), json!([201, {}]));
            f.responses(Value::Object(replies.clone()));
            upload(url, bytes).unwrap();
            let call = f.calls().pop().unwrap();
            assert_eq!(call["method"], create);
            assert_eq!(call["body"]["branch"], "backup/main");
            assert_eq!(call["body"]["content"], STANDARD.encode(bytes));
            assert!(call["args"].as_array().unwrap().contains(&json!(host)));
            assert!(
                !call["args"].to_string().contains("chat_logo"),
                "payload goes over stdin, not command arguments"
            );
            replies.insert(format!("GET {endpoint}"), json!([200, old]));
            replies.insert(format!("PUT {file}"), json!([200, {}]));
            f.responses(Value::Object(replies.clone()));
            assert_eq!(download(url).unwrap(), bytes);
            let changed =
                br#"{"format":"dunlin-backup","version":1,"settings":{"chat_logo":true}}"#;
            upload(url, changed).unwrap();
            let call = f.calls().pop().unwrap();
            assert_eq!(call["method"], "PUT");
            if github {
                assert_eq!(call["body"]["sha"], "revision");
            } else {
                assert_eq!(call["body"]["last_commit_id"], "commit");
                assert_eq!(call["body"]["encoding"], "base64");
            }
            replies.insert(
                format!("PUT {file}"),
                json!([409,{"message":"secret token"}]),
            );
            f.responses(Value::Object(replies.clone()));
            let before = f.calls().len();
            let error = upload(url, changed).unwrap_err();
            assert!(!error.contains("secret token"));
            assert_eq!(f.calls().len(), before + 3, "one write, never retried");
            replies.insert(format!("GET {endpoint}"), json!([403, {}]));
            f.responses(Value::Object(replies));
            let before = f.calls().len();
            assert!(upload(url, bytes).is_err());
            assert_eq!(
                f.calls().len(),
                before + 2,
                "authorization failures must not become file creation"
            );
        }
    }
    #[test]
    fn clone_refuses_existing_checkouts_and_host_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("github.com/o/r")).unwrap();
        std::fs::write(dir.path().join("github.com/o/r/keep"), "keep").unwrap();
        assert!(clone("https://github.com/o/r", dir.path()).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("github.com/o/r/keep")).unwrap(),
            "keep"
        );
        assert!(clone("https://gitlab.com/g/r", std::path::Path::new("relative")).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path(), dir.path().join("gitlab.com")).unwrap();
            assert!(clone("https://gitlab.com/g/r", dir.path()).is_err());
            assert!(!dir.path().join("g").exists());
        }
    }
}
