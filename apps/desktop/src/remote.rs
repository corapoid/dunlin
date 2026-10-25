//! Remote relay client and the local/remote workspace backend.
//!
//! The desktop talks to the relay over HTTP for pairing, project discovery,
//! sessions, files, search and git. ACP itself uses a separate WebSocket
//! connection managed by `dunlin-agent`.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use dunlin_domain::{ProviderUsage, RemoteProject};
use dunlin_workspace::fs::{FileContents, FileStat};
use dunlin_workspace::git::GitStatus;
use dunlin_workspace::search::SearchHit;
use dunlin_workspace::tree::FileTree;
use dunlin_workspace::{fs as workspace_fs, git, search};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const KEYCHAIN_SERVICE: &str = "dev.dunlin.remote";
const LEGACY_KEYCHAIN_SERVICE: &str = "dev.kestrel.remote";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteHost {
    pub id: String,
    pub name: String,
    pub url: String,
    pub user_name: String,
    pub device_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct RemoteUser {
    pub id: String,
    pub name: String,
    pub role: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct PairResponse {
    pub token: String,
    pub user: RemoteUser,
    pub device_id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RemoteProjectInfo {
    pub id: String,
    pub name: String,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct RemoteSessionInfo {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
    pub last_device_id: Option<String>,
    pub active_device_id: Option<String>,
}

pub(crate) fn pair(url: &str, code: &str, device_name: &str) -> Result<PairResponse, String> {
    let endpoint = format!("{}/v1/pair", url.trim_end_matches('/'));
    let response = ureq::post(&endpoint)
        .timeout(Duration::from_secs(15))
        .send_json(serde_json::json!({
            "code": code.trim().to_uppercase(),
            "device_name": device_name,
        }))
        .map_err(http_error)?;
    response.into_json().map_err(|error| error.to_string())
}

pub(crate) fn projects(url: &str, token: &str) -> Result<Vec<RemoteProjectInfo>, String> {
    let response = ureq::get(&format!("{}/v1/projects", url.trim_end_matches('/')))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(http_error)?;
    response.into_json().map_err(|error| error.to_string())
}

pub(crate) fn sessions(
    url: &str,
    token: &str,
    project_id: &str,
) -> Result<Vec<RemoteSessionInfo>, String> {
    let response = ureq::get(&format!(
        "{}/v1/sessions?project={}",
        url.trim_end_matches('/'),
        encode(project_id)
    ))
    .set("Authorization", &format!("Bearer {token}"))
    .timeout(Duration::from_secs(15))
    .call()
    .map_err(http_error)?;
    response.into_json().map_err(|error| error.to_string())
}

pub(crate) fn usage(url: &str, token: &str) -> Result<Vec<ProviderUsage>, String> {
    let response = ureq::get(&format!("{}/v1/usage", url.trim_end_matches('/')))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(http_error)?;
    response.into_json().map_err(|error| error.to_string())
}

pub(crate) fn save_transcript(
    url: &str,
    token: &str,
    session_id: &str,
    payload: &serde_json::Value,
) -> Result<(), String> {
    ureq::put(&format!(
        "{}/v1/sessions/{}/transcript",
        url.trim_end_matches('/'),
        encode(session_id)
    ))
    .set("Authorization", &format!("Bearer {token}"))
    .timeout(Duration::from_secs(30))
    .send_json(serde_json::json!({ "payload": payload }))
    .map(|_| ())
    .map_err(http_error)
}

pub(crate) fn delete_session(url: &str, token: &str, session_id: &str) -> Result<(), String> {
    ureq::delete(&format!(
        "{}/v1/sessions/{}",
        url.trim_end_matches('/'),
        encode(session_id)
    ))
    .set("Authorization", &format!("Bearer {token}"))
    .timeout(Duration::from_secs(15))
    .call()
    .map(|_| ())
    .map_err(http_error)
}

pub(crate) fn takeover_session(url: &str, token: &str, session_id: &str) -> Result<(), String> {
    ureq::post(&format!(
        "{}/v1/sessions/{}/takeover",
        url.trim_end_matches('/'),
        encode(session_id)
    ))
    .set("Authorization", &format!("Bearer {token}"))
    .timeout(Duration::from_secs(15))
    .send_string("")
    .map(|_| ())
    .map_err(http_error)
}

pub(crate) fn save_token(host_id: &str, token: &str) -> Result<(), String> {
    let status = Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            host_id,
            "-w",
            token,
        ])
        .status()
        .map_err(|error| format!("Could not access the macOS keychain: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("Could not save the relay token in the keychain".into())
    }
}

pub(crate) fn load_token(host_id: &str) -> Result<Option<String>, String> {
    if let Ok(token) = std::env::var("DUNLIN_REMOTE_TOKEN") {
        return Ok(Some(token));
    }
    if let Some(token) = read_token(KEYCHAIN_SERVICE, host_id)? {
        return Ok(Some(token));
    }
    let Some(token) = read_token(LEGACY_KEYCHAIN_SERVICE, host_id)? else {
        return Ok(None);
    };
    // Move tokens saved by the pre-rename build into the new service. A
    // failure here is harmless: the token is still returned for this launch.
    let _ = save_token(host_id, &token);
    delete_token_for(LEGACY_KEYCHAIN_SERVICE, host_id);
    Ok(Some(token))
}

fn read_token(service: &str, host_id: &str) -> Result<Option<String>, String> {
    let output = Command::new("security")
        .args(["find-generic-password", "-s", service, "-a", host_id, "-w"])
        .output()
        .map_err(|error| format!("Could not access the macOS keychain: {error}"))?;
    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ));
    }
    // `security` exits 44 when the item is missing; anything else is a real
    // error (for example a locked keychain).
    if output.status.code() == Some(44) {
        Ok(None)
    } else {
        Err("Could not read the relay token from the keychain".into())
    }
}

pub(crate) fn delete_token(host_id: &str) {
    delete_token_for(KEYCHAIN_SERVICE, host_id);
    delete_token_for(LEGACY_KEYCHAIN_SERVICE, host_id);
}

fn delete_token_for(service: &str, host_id: &str) {
    let _ = Command::new("security")
        .args(["delete-generic-password", "-s", service, "-a", host_id])
        .status();
}

#[derive(Clone)]
pub(crate) struct RemoteWorkspace {
    pub host: RemoteHost,
    pub project: RemoteProject,
    pub token: String,
}

impl RemoteWorkspace {
    pub fn tree(&self) -> Result<FileTree, String> {
        self.get(&format!(
            "/v1/fs/tree?project={}",
            encode(&self.project.project_id)
        ))
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
        self.get(&format!(
            "/v1/search?project={}&q={}&limit={}",
            encode(&self.project.project_id),
            encode(query),
            limit
        ))
    }

    pub fn git_status(&self) -> Result<GitStatus, String> {
        self.get(&format!(
            "/v1/git/status?project={}",
            encode(&self.project.project_id)
        ))
    }

    pub fn git_file_diff(&self, path: &str, staged: bool) -> Result<String, String> {
        #[derive(Deserialize)]
        struct Patch {
            patch: String,
        }
        let patch: Patch = self.get(&format!(
            "/v1/git/diff?project={}&file={}&staged={staged}",
            encode(&self.project.project_id),
            encode(&self.relative(path))
        ))?;
        Ok(patch.patch)
    }

    pub fn git_stage_file(&self, path: &str) -> Result<(), String> {
        self.post_empty(
            "/v1/git/stage",
            &serde_json::json!({
                "project": self.project.project_id,
                "file": self.relative(path),
            }),
        )
    }

    pub fn git_stage_all(&self) -> Result<(), String> {
        self.post_empty(
            "/v1/git/stage",
            &serde_json::json!({ "project": self.project.project_id }),
        )
    }

    pub fn git_unstage_file(&self, path: &str) -> Result<(), String> {
        self.post_empty(
            "/v1/git/unstage",
            &serde_json::json!({
                "project": self.project.project_id,
                "file": self.relative(path),
            }),
        )
    }

    pub fn git_commit(&self, message: &str) -> Result<(), String> {
        self.post_empty(
            "/v1/git/commit",
            &serde_json::json!({
                "project": self.project.project_id,
                "message": message,
            }),
        )
    }

    pub fn git_amend(&self, message: &str) -> Result<(), String> {
        self.post_empty(
            "/v1/git/amend",
            &serde_json::json!({
                "project": self.project.project_id,
                "message": message,
            }),
        )
    }

    pub fn git_discard(&self, path: &str) -> Result<(), String> {
        self.post_empty(
            "/v1/git/discard",
            &serde_json::json!({
                "project": self.project.project_id,
                "file": self.relative(path),
            }),
        )
    }

    pub fn git_pull(&self) -> Result<(), String> {
        self.post_empty(
            "/v1/git/pull",
            &serde_json::json!({ "project": self.project.project_id }),
        )
    }

    pub fn git_push(&self) -> Result<(), String> {
        self.post_empty(
            "/v1/git/push",
            &serde_json::json!({ "project": self.project.project_id }),
        )
    }

    pub fn git_branches(&self) -> Result<Vec<String>, String> {
        #[derive(Deserialize)]
        struct Branches {
            branches: Vec<String>,
        }
        let response: Branches = self.get(&format!(
            "/v1/git/branches?project={}",
            encode(&self.project.project_id)
        ))?;
        Ok(response.branches)
    }

    pub fn git_switch(&self, branch: &str) -> Result<(), String> {
        self.post_empty(
            "/v1/git/switch",
            &serde_json::json!({
                "project": self.project.project_id,
                "branch": branch,
            }),
        )
    }

    pub fn read_file(&self, path: &str) -> Result<FileContents, String> {
        self.get(&format!(
            "/v1/fs/file?project={}&path={}",
            encode(&self.project.project_id),
            encode(&self.relative(path))
        ))
    }

    pub fn write_file(
        &self,
        path: &str,
        contents: &str,
        expected_mtime_ms: Option<u64>,
    ) -> Result<FileStat, String> {
        self.put(
            "/v1/fs/file",
            &serde_json::json!({
                "project": self.project.project_id,
                "path": self.relative(path),
                "contents": contents,
                "expected_mtime_ms": expected_mtime_ms,
            }),
        )
    }

    pub fn create_file(&self, path: &str) -> Result<FileStat, String> {
        self.post(
            "/v1/fs/create",
            &serde_json::json!({
                "project": self.project.project_id,
                "path": self.relative(path),
            }),
        )
    }

    pub fn stat_file(&self, path: &str) -> Result<FileStat, String> {
        self.get(&format!(
            "/v1/fs/stat?project={}&path={}",
            encode(&self.project.project_id),
            encode(&self.relative(path))
        ))
    }

    /// Upload a local attachment into the remote project and return its
    /// project-relative path.
    pub fn upload_file(&self, local_path: &Path) -> Result<String, String> {
        #[derive(Deserialize)]
        struct Uploaded {
            path: String,
        }
        let bytes = std::fs::read(local_path)
            .map_err(|error| format!("Could not read {}: {error}", local_path.display()))?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("Attachments must be smaller than 8 MiB".into());
        }
        let name = local_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("attachment");
        let response: Uploaded = self.post(
            "/v1/fs/upload",
            &serde_json::json!({
                "project": self.project.project_id,
                "name": name,
                "contents": STANDARD.encode(bytes),
            }),
        )?;
        Ok(response.path)
    }

    fn relative(&self, path: &str) -> String {
        Path::new(path)
            .strip_prefix(&self.project.path)
            .map(|relative| {
                relative
                    .to_string_lossy()
                    .trim_start_matches('/')
                    .to_string()
            })
            .unwrap_or_else(|_| path.trim_start_matches('/').to_string())
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let response = ureq::get(&format!("{}{}", self.host.url.trim_end_matches('/'), path))
            .set("Authorization", &format!("Bearer {}", self.token))
            .timeout(Duration::from_secs(30))
            .call()
            .map_err(http_error)?;
        response.into_json().map_err(|error| error.to_string())
    }

    fn put<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T, String> {
        let response = ureq::put(&format!("{}{}", self.host.url.trim_end_matches('/'), path))
            .set("Authorization", &format!("Bearer {}", self.token))
            .timeout(Duration::from_secs(30))
            .send_json(body)
            .map_err(http_error)?;
        response.into_json().map_err(|error| error.to_string())
    }

    fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T, String> {
        let response = ureq::post(&format!("{}{}", self.host.url.trim_end_matches('/'), path))
            .set("Authorization", &format!("Bearer {}", self.token))
            .timeout(Duration::from_secs(30))
            .send_json(body)
            .map_err(http_error)?;
        response.into_json().map_err(|error| error.to_string())
    }

    fn post_empty(&self, path: &str, body: &Value) -> Result<(), String> {
        ureq::post(&format!("{}{}", self.host.url.trim_end_matches('/'), path))
            .set("Authorization", &format!("Bearer {}", self.token))
            .timeout(Duration::from_secs(30))
            .send_json(body)
            .map(|_| ())
            .map_err(http_error)
    }
}

#[derive(Clone)]
pub(crate) enum WorkspaceBackend {
    Local { root: String },
    Remote(RemoteWorkspace),
}

impl WorkspaceBackend {
    pub fn tree(&self) -> Result<FileTree, String> {
        match self {
            WorkspaceBackend::Local { root } => Ok(FileTree::load(root.clone())),
            WorkspaceBackend::Remote(remote) => remote.tree(),
        }
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
        match self {
            WorkspaceBackend::Local { root } => Ok(search::search_project(root, query, limit)),
            WorkspaceBackend::Remote(remote) => remote.search(query, limit),
        }
    }

    pub fn git_status(&self) -> Result<GitStatus, String> {
        match self {
            WorkspaceBackend::Local { root } => Ok(git::load(root)),
            WorkspaceBackend::Remote(remote) => remote.git_status(),
        }
    }

    pub fn git_file_diff(&self, path: &str, staged: bool) -> Result<String, String> {
        match self {
            WorkspaceBackend::Local { root } => {
                git::file_diff(root, &self.relative(root, path), staged)
            }
            WorkspaceBackend::Remote(remote) => remote.git_file_diff(path, staged),
        }
    }

    pub fn git_stage_file(&self, path: &str) -> Result<(), String> {
        match self {
            WorkspaceBackend::Local { root } => git::stage_file(root, &self.relative(root, path)),
            WorkspaceBackend::Remote(remote) => remote.git_stage_file(path),
        }
    }

    pub fn git_stage_all(&self) -> Result<(), String> {
        match self {
            WorkspaceBackend::Local { root } => git::stage_all(root),
            WorkspaceBackend::Remote(remote) => remote.git_stage_all(),
        }
    }

    pub fn git_unstage_file(&self, path: &str) -> Result<(), String> {
        match self {
            WorkspaceBackend::Local { root } => git::unstage_file(root, &self.relative(root, path)),
            WorkspaceBackend::Remote(remote) => remote.git_unstage_file(path),
        }
    }

    pub fn git_commit(&self, message: &str) -> Result<(), String> {
        match self {
            WorkspaceBackend::Local { root } => git::commit(root, message),
            WorkspaceBackend::Remote(remote) => remote.git_commit(message),
        }
    }

    pub fn git_amend(&self, message: &str) -> Result<(), String> {
        match self {
            WorkspaceBackend::Local { root } => git::amend(root, message),
            WorkspaceBackend::Remote(remote) => remote.git_amend(message),
        }
    }

    pub fn git_discard(&self, path: &str) -> Result<(), String> {
        match self {
            WorkspaceBackend::Local { root } => git::discard_file(root, &self.relative(root, path)),
            WorkspaceBackend::Remote(remote) => remote.git_discard(path),
        }
    }

    pub fn git_pull(&self) -> Result<(), String> {
        match self {
            WorkspaceBackend::Local { root } => git::pull(root),
            WorkspaceBackend::Remote(remote) => remote.git_pull(),
        }
    }

    pub fn git_push(&self) -> Result<(), String> {
        match self {
            WorkspaceBackend::Local { root } => git::push(root),
            WorkspaceBackend::Remote(remote) => remote.git_push(),
        }
    }

    pub fn git_branches(&self) -> Result<Vec<String>, String> {
        match self {
            WorkspaceBackend::Local { root } => git::branches(root),
            WorkspaceBackend::Remote(remote) => remote.git_branches(),
        }
    }

    pub fn git_switch(&self, branch: &str) -> Result<(), String> {
        match self {
            WorkspaceBackend::Local { root } => git::switch_branch(root, branch),
            WorkspaceBackend::Remote(remote) => remote.git_switch(branch),
        }
    }

    pub fn read_file(&self, path: &str) -> Result<FileContents, String> {
        match self {
            WorkspaceBackend::Local { root } => {
                workspace_fs::read_file(root, &self.relative(root, path))
            }
            WorkspaceBackend::Remote(remote) => remote.read_file(path),
        }
    }

    pub fn write_file(
        &self,
        path: &str,
        contents: &str,
        expected_mtime_ms: Option<u64>,
    ) -> Result<FileStat, String> {
        match self {
            WorkspaceBackend::Local { root } => {
                let relative = self.relative(root, path);
                if let Some(expected) = expected_mtime_ms {
                    let current = workspace_fs::stat_file(root, &relative)?;
                    if current.mtime_ms != expected {
                        return Err("conflict: file changed on disk since it was opened".into());
                    }
                }
                workspace_fs::write_file(root, &relative, contents)
            }
            WorkspaceBackend::Remote(remote) => {
                remote.write_file(path, contents, expected_mtime_ms)
            }
        }
    }

    pub fn create_file(&self, path: &str) -> Result<FileStat, String> {
        match self {
            WorkspaceBackend::Local { root } => {
                workspace_fs::create_file(root, &self.relative(root, path))
            }
            WorkspaceBackend::Remote(remote) => remote.create_file(path),
        }
    }

    pub fn stat_file(&self, path: &str) -> Result<FileStat, String> {
        match self {
            WorkspaceBackend::Local { root } => {
                workspace_fs::stat_file(root, &self.relative(root, path))
            }
            WorkspaceBackend::Remote(remote) => remote.stat_file(path),
        }
    }

    pub fn upload_file(&self, path: &Path) -> Result<String, String> {
        match self {
            WorkspaceBackend::Local { .. } => Ok(path.to_string_lossy().to_string()),
            WorkspaceBackend::Remote(remote) => remote.upload_file(path),
        }
    }

    fn relative(&self, root: &str, path: &str) -> String {
        Path::new(path)
            .strip_prefix(root)
            .map(|relative| {
                relative
                    .to_string_lossy()
                    .trim_start_matches('/')
                    .to_string()
            })
            .unwrap_or_else(|_| path.trim_start_matches('/').to_string())
    }
}

fn http_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            let message = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|value| {
                    value
                        .get("error")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or(body);
            if code == 409 {
                format!("conflict: {message}")
            } else {
                format!("Relay error {code}: {message}")
            }
        }
        ureq::Error::Transport(error) => format!("Relay connection failed: {error}"),
    }
}

fn encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::{WorkspaceBackend, encode};
    use std::fs;

    #[test]
    fn encodes_query_values() {
        assert_eq!(encode("src/main file.rs"), "src/main%20file.rs");
        assert_eq!(encode("a&b"), "a%26b");
    }

    #[test]
    fn local_backend_reads_files() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dunlin-backend-{unique}"));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("hello.txt"), "hi").unwrap();
        let backend = WorkspaceBackend::Local {
            root: root.to_string_lossy().to_string(),
        };
        let file = backend
            .read_file(&root.join("hello.txt").to_string_lossy())
            .unwrap();
        assert_eq!(file.contents, "hi");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn local_backend_blocks_stale_writes() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dunlin-conflict-{unique}"));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("hello.txt");
        fs::write(&path, "one").unwrap();
        let backend = WorkspaceBackend::Local {
            root: root.to_string_lossy().to_string(),
        };
        let path_text = path.to_string_lossy().to_string();
        let stat = backend.stat_file(&path_text).unwrap();
        let error = backend
            .write_file(&path_text, "two", Some(stat.mtime_ms + 1))
            .unwrap_err();
        assert!(error.starts_with("conflict:"), "{error}");
        backend
            .write_file(&path_text, "two", Some(stat.mtime_ms))
            .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let _ = fs::remove_dir_all(&root);
    }
}
