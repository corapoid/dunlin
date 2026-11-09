//! Remote relay client and the local/remote workspace backend.
//!
//! The desktop talks to the relay over HTTP for pairing, project discovery,
//! sessions, files, search and git. ACP itself uses a separate WebSocket
//! connection managed by `dunlin-agent`.

use std::path::Path;
use std::time::Duration;

use url::Url;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use dunlin_domain::{ProjectSettings, ProviderUsage, RemoteProject};
use dunlin_workspace::checks;
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

mod history;
pub(crate) mod operations;
pub(crate) mod transcripts;

fn explorer_relay_error(error: String) -> String {
    if error.starts_with("Relay error 404:") || error.starts_with("Relay error 405:") {
        "This Explorer action requires an updated Dunlin relay. Update the relay and try again."
            .into()
    } else {
        error
    }
}

fn entry_relative(root: &str, path: &str) -> Result<String, String> {
    let path = Path::new(path);
    let relative = if path.is_absolute() {
        path.strip_prefix(root)
            .map_err(|_| "The item is outside the selected project")?
    } else {
        path
    };
    if relative.components().any(|part| {
        !matches!(
            part,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    }) {
        return Err("The item is outside the selected project".into());
    }
    Ok(relative.to_string_lossy().to_string())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteHost {
    pub id: String,
    pub name: String,
    pub url: String,
    pub user_name: String,
    #[serde(default)]
    pub user_role: String,
    pub device_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RemoteArtifact {
    pub id: String,
    pub project_id: String,
    pub kind: String,
    pub title: String,
    pub mime: String,
    pub bytes: i64,
    pub sha256: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RemoteArtifactShare {
    pub token: String,
    pub expires_at: i64,
    pub url: String,
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
pub(crate) struct RemoteAuditRun {
    pub id: String,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RemoteRestorePreview {
    pub id: String,
    pub change: dunlin_workspace::task_review::Change,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RemotePreparedComparison {
    pub id: String,
    pub head: String,
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RemoteComparisonTrees {
    pub trees: dunlin_workspace::comparison::Trees,
    pub projects: Vec<RemoteProjectInfo>,
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

/// Relay URLs must be HTTPS, or plain HTTP only on loopback. A self-hosted
/// relay on Tailscale normally terminates TLS; refusing clear-text LAN URLs
/// keeps pairing codes, bearer tokens and prompts off the wire.
use dunlin_http as ureq;

fn http_agent() -> ureq::Agent {
    // A relay bearer token must never be forwarded through an environment
    // proxy by accident.
    ureq::AgentBuilder::new().try_proxy_from_env(false).build()
}

pub(crate) fn validate_relay_url(url: &str) -> Result<Url, String> {
    let url = Url::parse(url.trim()).map_err(|_| "Invalid relay URL".to_string())?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Relay URL must be an http(s) URL without credentials".into());
    }
    if url.scheme() == "http" && !loopback {
        return Err(
            "Plain HTTP is only allowed for loopback. Use HTTPS (for example Tailscale Serve) for a remote relay."
                .into(),
        );
    }
    Ok(url)
}

pub(crate) fn pair(url: &str, code: &str, device_name: &str) -> Result<PairResponse, String> {
    validate_relay_url(url)?;
    let endpoint = format!("{}/v1/pair", url.trim_end_matches('/'));
    let response = http_agent()
        .post(&endpoint)
        .timeout(Duration::from_secs(15))
        .send_json(serde_json::json!({
            "code": code.trim().to_uppercase(),
            "device_name": device_name,
        }))
        .map_err(http_error)?;
    response.into_json().map_err(|error| error.to_string())
}

pub(crate) fn projects(url: &str, token: &str) -> Result<Vec<RemoteProjectInfo>, String> {
    validate_relay_url(url)?;
    let response = http_agent()
        .get(&format!("{}/v1/projects", url.trim_end_matches('/')))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(http_error)?;
    response.into_json().map_err(|error| error.to_string())
}

pub(crate) fn run_check(
    url: &str,
    token: &str,
    project_id: &str,
    argv: Vec<String>,
) -> Result<checks::Report, String> {
    validate_relay_url(url)?;
    let response = http_agent()
        .post(&format!("{}/v1/checks/run", url.trim_end_matches('/')))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(150))
        .send_json(serde_json::json!({"project": project_id, "argv": argv}))
        .map_err(http_error)?;
    response.into_json().map_err(|error| error.to_string())
}

pub(crate) fn build_impact(
    url: &str,
    token: &str,
    project_id: &str,
) -> Result<dunlin_workspace::impact::Index, String> {
    validate_relay_url(url)?;
    let response = http_agent()
        .post(&format!("{}/v1/impact/build", url.trim_end_matches('/')))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(60))
        .send_json(serde_json::json!({"project": project_id}))
        .map_err(http_error)?;
    response.into_json().map_err(|error| error.to_string())
}

pub(crate) fn verify_impact(
    url: &str,
    token: &str,
    project_id: &str,
    index: &dunlin_workspace::impact::Index,
) -> Result<(), String> {
    validate_relay_url(url)?;
    http_agent()
        .post(&format!("{}/v1/impact/verify", url.trim_end_matches('/')))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(30))
        .send_json(serde_json::json!({"project": project_id, "index": index}))
        .map(|_| ())
        .map_err(http_error)
}

pub(crate) fn project_settings(
    url: &str,
    token: &str,
    project_id: &str,
) -> Result<ProjectSettings, String> {
    validate_relay_url(url)?;
    let response = http_agent()
        .get(&format!(
            "{}/v1/project/settings?project={}",
            url.trim_end_matches('/'),
            encode(project_id)
        ))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(http_error)?;
    response.into_json().map_err(|error| error.to_string())
}

pub(crate) fn coordination(
    url: &str,
    token: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    validate_relay_url(url)?;
    let response = http_agent()
        .post(&format!("{}/v1/coordination", url.trim_end_matches('/')))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(15))
        .send_json(body)
        .map_err(http_error)?;
    let mut bytes = Vec::new();
    use std::io::Read;
    response
        .into_reader()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Relay coordination response exceeds 8 MiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

pub(crate) fn save_project_settings(
    url: &str,
    token: &str,
    project_id: &str,
    settings: &ProjectSettings,
) -> Result<(), String> {
    validate_relay_url(url)?;
    http_agent()
        .put(&format!(
            "{}/v1/project/settings",
            url.trim_end_matches('/')
        ))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(15))
        .send_json(serde_json::json!({
            "project": project_id,
            "settings": settings,
        }))
        .map(|_| ())
        .map_err(http_error)
}

pub(crate) fn sessions(
    url: &str,
    token: &str,
    project_id: &str,
) -> Result<Vec<RemoteSessionInfo>, String> {
    validate_relay_url(url)?;
    let response = http_agent()
        .get(&format!(
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

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct RemoteBillingEvidence {
    pub user_id: String,
    pub user_name: String,
    pub device_id: String,
    pub device_name: String,
    pub updated_at: i64,
    pub evidence: dunlin_usage::billing::Evidence,
}

pub(crate) fn sync_billing_evidence(
    url: &str,
    token: &str,
    evidence: &dunlin_usage::billing::Evidence,
) -> Result<(), String> {
    validate_relay_url(url)?;
    evidence.validate()?;
    http_agent()
        .post(&format!(
            "{}/v1/billing/evidence",
            url.trim_end_matches('/')
        ))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(30))
        .send_json(evidence)
        .map(|_| ())
        .map_err(http_error)
}

pub(crate) fn billing_evidence(
    url: &str,
    token: &str,
) -> Result<Vec<RemoteBillingEvidence>, String> {
    validate_relay_url(url)?;
    let response = http_agent()
        .get(&format!(
            "{}/v1/billing/evidence",
            url.trim_end_matches('/')
        ))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(http_error)?;
    let rows: Vec<RemoteBillingEvidence> =
        response.into_json().map_err(|error| error.to_string())?;
    for row in &rows {
        row.evidence.validate()?;
    }
    Ok(rows)
}

pub(crate) fn usage(url: &str, token: &str) -> Result<Vec<ProviderUsage>, String> {
    validate_relay_url(url)?;
    let response = http_agent()
        .get(&format!("{}/v1/usage", url.trim_end_matches('/')))
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
    expected_revision: i64,
) -> Result<(), String> {
    validate_relay_url(url)?;
    http_agent()
        .put(&format!(
            "{}/v1/sessions/{}/transcript",
            url.trim_end_matches('/'),
            encode(session_id)
        ))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(30))
        .send_json(
            serde_json::json!({ "payload": payload, "expected_revision": expected_revision }),
        )
        .map(|_| ())
        .map_err(http_error)
}

fn transcript_revision(url: &str, token: &str, session_id: &str) -> Result<i64, String> {
    validate_relay_url(url)?;
    let response = http_agent()
        .get(&format!(
            "{}/v1/sessions/{}/transcript/revision",
            url.trim_end_matches('/'),
            encode(session_id)
        ))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(http_error)?;
    #[derive(Deserialize)]
    struct Revision {
        revision: i64,
    }
    let value: Revision = response.into_json().map_err(|error| error.to_string())?;
    if value.revision < 0 || value.revision == i64::MAX {
        return Err("Invalid relay transcript revision".into());
    }
    Ok(value.revision)
}

pub(crate) fn delete_session(url: &str, token: &str, session_id: &str) -> Result<(), String> {
    validate_relay_url(url)?;
    http_agent()
        .delete(&format!(
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
    validate_relay_url(url)?;
    http_agent()
        .post(&format!(
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

#[cfg(target_os = "macos")]
pub(crate) fn save_token(host_id: &str, token: &str) -> Result<(), String> {
    security_framework::passwords::set_generic_password(KEYCHAIN_SERVICE, host_id, token.as_bytes())
        .map_err(|error| format!("Could not save the relay token in the keychain: {error}"))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn save_token(_host_id: &str, _token: &str) -> Result<(), String> {
    Err("Relay token storage requires macOS Keychain".into())
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

#[cfg(target_os = "macos")]
fn read_token(service: &str, host_id: &str) -> Result<Option<String>, String> {
    use security_framework::passwords::{PasswordOptions, generic_password};

    match generic_password(PasswordOptions::new_generic_password(service, host_id)) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| "The relay token in the keychain is not valid UTF-8".into()),
        // errSecItemNotFound. Anything else (for example a locked keychain) is
        // a real error and must not be treated as "not paired".
        Err(error) if error.code() == -25300 => Ok(None),
        Err(error) => Err(format!(
            "Could not read the relay token from the keychain: {error}"
        )),
    }
}

#[cfg(not(target_os = "macos"))]
fn read_token(_service: &str, _host_id: &str) -> Result<Option<String>, String> {
    Err("Relay token storage requires macOS Keychain".into())
}

pub(crate) fn delete_token(host_id: &str) {
    delete_token_for(KEYCHAIN_SERVICE, host_id);
    delete_token_for(LEGACY_KEYCHAIN_SERVICE, host_id);
}

/// Factory reset must report Keychain failures instead of forgetting their IDs.
pub(crate) fn delete_token_checked(host_id: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    for service in [KEYCHAIN_SERVICE, LEGACY_KEYCHAIN_SERVICE] {
        match security_framework::passwords::delete_generic_password(service, host_id) {
            Ok(()) => {}
            Err(error) if error.code() == -25300 => {}
            Err(_) => return Err("Could not remove a relay pairing from the Keychain. Unlock the Keychain and retry.".into()),
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = host_id;
    Ok(())
}

#[cfg(target_os = "macos")]
fn delete_token_for(service: &str, host_id: &str) {
    let _ = security_framework::passwords::delete_generic_password(service, host_id);
}

#[cfg(not(target_os = "macos"))]
fn delete_token_for(_service: &str, _host_id: &str) {}

#[derive(Clone)]
pub(crate) struct RemoteWorkspace {
    pub host: RemoteHost,
    pub project: RemoteProject,
    pub token: String,
}

impl RemoteWorkspace {
    pub(crate) fn storage_size(&self) -> Result<Option<u64>, String> {
        self.get(&format!(
            "/v1/fs/storage?project={}",
            encode(&self.project.project_id)
        ))
    }
    pub fn tree(&self) -> Result<FileTree, String> {
        self.get(&format!(
            "/v1/fs/tree?project={}",
            encode(&self.project.project_id)
        ))
    }

    pub fn audit_runs(&self) -> Result<Vec<RemoteAuditRun>, String> {
        self.get(&format!(
            "/v1/audits?project={}",
            encode(&self.project.project_id)
        ))
    }

    pub fn audit_file(&self, run: &str, name: &str) -> Result<FileContents, String> {
        self.get(&format!(
            "/v1/audits/file?project={}&run={}&name={}",
            encode(&self.project.project_id),
            encode(run),
            encode(name)
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
            encode(&entry_relative(&self.project.path, path)?)
        ))?;
        Ok(patch.patch)
    }

    pub fn git_stage_file(&self, path: &str) -> Result<(), String> {
        self.post_empty(
            "/v1/git/stage",
            &serde_json::json!({
                "project": self.project.project_id,
                "file": entry_relative(&self.project.path, path)?,
            }),
        )
    }

    pub fn git_stage_all(&self) -> Result<(), String> {
        self.post_empty(
            "/v1/git/stage",
            &serde_json::json!({ "project": self.project.project_id }),
        )
    }

    pub fn git_initialize(&self) -> Result<(), String> {
        self.post_empty(
            "/v1/git/init",
            &serde_json::json!({"project": self.project.project_id}),
        )
        .map_err(explorer_relay_error)
    }

    pub fn git_unstage_file(&self, path: &str) -> Result<(), String> {
        self.post_empty(
            "/v1/git/unstage",
            &serde_json::json!({
                "project": self.project.project_id,
                "file": entry_relative(&self.project.path, path)?,
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

    pub fn read_file(&self, path: &str) -> Result<FileContents, String> {
        self.get(&format!(
            "/v1/fs/file?project={}&path={}",
            encode(&self.project.project_id),
            encode(&entry_relative(&self.project.path, path)?)
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
                "path": entry_relative(&self.project.path, path)?,
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
                "path": entry_relative(&self.project.path, path)?,
            }),
        )
    }

    pub fn create_directory(&self, path: &str) -> Result<FileStat, String> {
        self.post("/v1/fs/mkdir", &serde_json::json!({"project": self.project.project_id, "path": entry_relative(&self.project.path, path)?})).map_err(explorer_relay_error)
    }

    pub fn move_entries(
        &self,
        sources: &[String],
        target: &str,
    ) -> Result<dunlin_workspace::transfer::TransferReport, String> {
        let sources = sources
            .iter()
            .map(|path| entry_relative(&self.project.path, path))
            .collect::<Result<Vec<_>, _>>()?;
        self.post("/v1/fs/move", &serde_json::json!({"project": self.project.project_id, "sources": sources, "target": entry_relative(&self.project.path, target)?})).map_err(explorer_relay_error)
    }

    pub fn stat_file(&self, path: &str) -> Result<FileStat, String> {
        self.get(&format!(
            "/v1/fs/stat?project={}&path={}",
            encode(&self.project.project_id),
            encode(&entry_relative(&self.project.path, path)?)
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

    pub fn compare_prepare(&self) -> Result<RemotePreparedComparison, String> {
        self.post(
            "/v1/compare/prepare",
            &serde_json::json!({"project": self.project.project_id}),
        )
    }

    pub fn compare_create(&self, id: &str) -> Result<RemoteComparisonTrees, String> {
        self.post(
            "/v1/compare/create",
            &serde_json::json!({"project": self.project.project_id, "id": id}),
        )
    }

    pub fn compare_changes(
        &self,
        checkpoint: &dunlin_workspace::task_review::Checkpoint,
    ) -> Result<Vec<dunlin_workspace::task_review::Change>, String> {
        self.post(
            "/v1/compare/changes",
            &serde_json::json!({"project": self.project.project_id, "checkpoint": checkpoint}),
        )
    }

    pub fn compare_validate(&self, head: &str) -> Result<(), String> {
        self.post_empty(
            "/v1/compare/validate",
            &serde_json::json!({"project": self.project.project_id, "head": head}),
        )
    }

    pub(crate) fn github_metadata(&self) -> Result<dunlin_workspace::github::Metadata, String> {
        self.post(
            "/v1/github/metadata",
            &serde_json::json!({"project": self.project.project_id, "remote": "origin"}),
        )
    }

    pub fn review_capture(
        &self,
        label: &str,
    ) -> Result<dunlin_workspace::task_review::Checkpoint, String> {
        self.post(
            "/v1/review/capture",
            &serde_json::json!({"project": self.project.project_id, "label": label}),
        )
    }

    pub fn review_changes(
        &self,
        checkpoint: &dunlin_workspace::task_review::Checkpoint,
    ) -> Result<Vec<dunlin_workspace::task_review::Change>, String> {
        self.post(
            "/v1/review/changes",
            &serde_json::json!({"project": self.project.project_id, "checkpoint": checkpoint}),
        )
    }

    pub fn review_restore_prepare(
        &self,
        checkpoint: &dunlin_workspace::task_review::Checkpoint,
        path: &str,
    ) -> Result<RemoteRestorePreview, String> {
        self.post(
            "/v1/review/restore/prepare",
            &serde_json::json!({"project": self.project.project_id, "checkpoint": checkpoint, "path": path}),
        )
    }

    pub fn review_restore_execute(&self, id: &str) -> Result<(), String> {
        self.post_empty(
            "/v1/review/restore/execute",
            &serde_json::json!({"project": self.project.project_id, "id": id}),
        )
    }

    pub fn remote_context(&self) -> Result<Value, String> {
        self.get(&format!(
            "/v1/context?project={}",
            encode(&self.project.project_id)
        ))
    }

    pub fn artifact_list(&self) -> Result<Vec<RemoteArtifact>, String> {
        self.get(&format!(
            "/v1/artifacts?project={}",
            encode(&self.project.project_id)
        ))
    }

    pub fn upload_artifact(
        &self,
        kind: &str,
        title: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Result<RemoteArtifact, String> {
        self.post(
            "/v1/artifacts",
            &serde_json::json!({
                "project_id": self.project.project_id,
                "kind": kind,
                "title": title,
                "mime": mime,
                "data": STANDARD.encode(bytes),
            }),
        )
    }

    pub fn download_artifact(&self, id: &str) -> Result<Vec<u8>, String> {
        #[derive(Deserialize)]
        struct Payload {
            data: String,
        }
        let payload: Payload = self.get(&format!("/v1/artifacts/{id}"))?;
        STANDARD
            .decode(payload.data.as_bytes())
            .map_err(|_| "Relay returned invalid artifact base64".into())
    }

    pub fn share_artifact(&self, id: &str, ttl_secs: u64) -> Result<RemoteArtifactShare, String> {
        self.post(
            &format!("/v1/artifacts/{id}/share"),
            &serde_json::json!({ "ttl_secs": ttl_secs }),
        )
    }

    pub fn delete_artifact(&self, id: &str) -> Result<(), String> {
        validate_relay_url(&self.host.url)?;
        http_agent()
            .delete(&format!(
                "{}/v1/artifacts/{id}",
                self.host.url.trim_end_matches('/')
            ))
            .set("Authorization", &format!("Bearer {}", self.token))
            .timeout(Duration::from_secs(30))
            .call()
            .map(|_| ())
            .map_err(http_error)
    }

    pub fn revoke_artifact_shares(&self, id: &str) -> Result<(), String> {
        validate_relay_url(&self.host.url)?;
        http_agent()
            .delete(&format!(
                "{}/v1/artifacts/{id}/share",
                self.host.url.trim_end_matches('/')
            ))
            .set("Authorization", &format!("Bearer {}", self.token))
            .timeout(Duration::from_secs(30))
            .call()
            .map(|_| ())
            .map_err(http_error)
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        validate_relay_url(&self.host.url)?;
        let response = http_agent()
            .get(&format!("{}{}", self.host.url.trim_end_matches('/'), path))
            .set("Authorization", &format!("Bearer {}", self.token))
            .timeout(Duration::from_secs(30))
            .call()
            .map_err(http_error)?;
        response.into_json().map_err(|error| error.to_string())
    }

    fn put<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T, String> {
        validate_relay_url(&self.host.url)?;
        let response = http_agent()
            .put(&format!("{}{}", self.host.url.trim_end_matches('/'), path))
            .set("Authorization", &format!("Bearer {}", self.token))
            .timeout(Duration::from_secs(30))
            .send_json(body)
            .map_err(http_error)?;
        response.into_json().map_err(|error| error.to_string())
    }

    fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T, String> {
        validate_relay_url(&self.host.url)?;
        let response = http_agent()
            .post(&format!("{}{}", self.host.url.trim_end_matches('/'), path))
            .set("Authorization", &format!("Bearer {}", self.token))
            .timeout(Duration::from_secs(30))
            .send_json(body)
            .map_err(http_error)?;
        response.into_json().map_err(|error| error.to_string())
    }

    fn post_empty(&self, path: &str, body: &Value) -> Result<(), String> {
        validate_relay_url(&self.host.url)?;
        http_agent()
            .post(&format!("{}{}", self.host.url.trim_end_matches('/'), path))
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
    Remote(Box<RemoteWorkspace>),
}

impl WorkspaceBackend {
    pub fn artifact_list(&self) -> Result<Vec<RemoteArtifact>, String> {
        match self {
            Self::Remote(remote) => remote.artifact_list(),
            Self::Local { .. } => Err("Artifacts are local-only here".into()),
        }
    }

    pub fn upload_artifact(
        &self,
        kind: &str,
        title: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Result<RemoteArtifact, String> {
        match self {
            Self::Remote(remote) => remote.upload_artifact(kind, title, mime, bytes),
            Self::Local { .. } => Err("Artifacts are local-only here".into()),
        }
    }

    pub fn download_artifact(&self, id: &str) -> Result<Vec<u8>, String> {
        match self {
            Self::Remote(remote) => remote.download_artifact(id),
            Self::Local { .. } => Err("Artifacts are local-only here".into()),
        }
    }

    pub fn share_artifact(&self, id: &str, ttl_secs: u64) -> Result<RemoteArtifactShare, String> {
        match self {
            Self::Remote(remote) => remote.share_artifact(id, ttl_secs),
            Self::Local { .. } => Err("Pair a relay to share an artifact".into()),
        }
    }

    pub fn revoke_artifact_shares(&self, id: &str) -> Result<(), String> {
        match self {
            Self::Remote(remote) => remote.revoke_artifact_shares(id),
            Self::Local { .. } => Err("Pair a relay to revoke an artifact share".into()),
        }
    }

    pub fn delete_artifact(&self, id: &str) -> Result<(), String> {
        match self {
            Self::Remote(remote) => remote.delete_artifact(id),
            Self::Local { .. } => Err("Artifacts are local-only here".into()),
        }
    }

    pub fn branch_context(&self) -> Result<git::BranchContext, String> {
        match self {
            Self::Local { root } => git::branch_context(root),
            Self::Remote(remote) => remote
                .get(&format!(
                    "/v1/git/branch-context?project={}",
                    encode(&remote.project.project_id)
                ))
                .map_err(branch_relay_error),
        }
    }
    /// Create one ref from an exact commit on the local or relay repository.
    /// Never checks out, fetches or moves the current branch.
    pub fn create_branch_at(&self, scope: &str, oid: &str, name: &str) -> Result<(), String> {
        match self {
            Self::Local { root } => {
                let root = git::history::repository_root(root, Some(scope))?;
                git::history::create_branch(&root, name, oid)
            }
            Self::Remote(remote) => remote
                .post_empty(
                    "/v1/git/history/branch",
                    &serde_json::json!({
                        "project": remote.project.project_id,
                        "scope": scope,
                        "oid": oid,
                        "name": name,
                    }),
                )
                .map_err(branch_relay_error),
        }
    }

    pub fn editor_baseline(&self, path: &str) -> Option<String> {
        match self {
            Self::Local { root } => git::head_file(root, &entry_relative(root, path).ok()?)
                .ok()
                .flatten(),
            Self::Remote(_) => None, // No extra relay capability or secret file reads.
        }
    }
    pub fn preview_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
        match self {
            Self::Local { root } => {
                dunlin_workspace::preview::read(root, &entry_relative(root, path)?)
            }
            Self::Remote(remote) => {
                let value: Value = remote
                    .get(&format!(
                        "/v1/fs/preview?project={}&path={}",
                        encode(&remote.project.project_id),
                        encode(&entry_relative(&remote.project.path, path)?)
                    ))
                    .map_err(explorer_relay_error)?;
                let encoded = value
                    .get("contents")
                    .and_then(Value::as_str)
                    .ok_or("Invalid preview response")?;
                if encoded.len()
                    > (dunlin_workspace::preview::MAX_PREVIEW_BYTES as usize).div_ceil(3) * 4
                {
                    return Err("Preview response is too large".into());
                }
                STANDARD
                    .decode(encoded)
                    .map_err(|_| "Invalid preview encoding".into())
            }
        }
    }
    pub fn rename_entry(
        &self,
        path: &str,
        name: &str,
    ) -> Result<dunlin_workspace::transfer::Transferred, String> {
        match self {
            Self::Local { root } => dunlin_workspace::transfer::rename_entry(root, &entry_relative(root, path)?, name),
            Self::Remote(remote) => remote.post("/v1/fs/rename", &serde_json::json!({"project": remote.project.project_id, "path": entry_relative(&remote.project.path, path)?, "name": name})).map_err(explorer_relay_error),
        }
    }
    pub fn delete_entry(&self, path: &str) -> Result<(), String> {
        match self {
            Self::Local { root } => {
                let path = dunlin_workspace::transfer::mutable_entry(root, &entry_relative(root, path)?)?;
                crate::platform::trash(&path)
            }
            Self::Remote(remote) => remote.post_empty("/v1/fs/delete", &serde_json::json!({"project": remote.project.project_id, "path": entry_relative(&remote.project.path, path)?})).map_err(explorer_relay_error),
        }
    }
    pub fn tree(&self) -> Result<FileTree, String> {
        match self {
            WorkspaceBackend::Local { root } => Ok(FileTree::load(root.clone())),
            WorkspaceBackend::Remote(remote) => remote.tree(),
        }
    }

    pub fn audit_runs(&self) -> Result<Vec<RemoteAuditRun>, String> {
        match self {
            WorkspaceBackend::Local { .. } => Err("audit artifacts are local-only here".into()),
            WorkspaceBackend::Remote(remote) => remote.audit_runs(),
        }
    }

    pub fn audit_file(&self, run: &str, name: &str) -> Result<FileContents, String> {
        match self {
            WorkspaceBackend::Local { root } => {
                let relative = format!(".dunlin/audits/{run}/{name}");
                workspace_fs::read_file(root, &relative)
            }
            WorkspaceBackend::Remote(remote) => remote.audit_file(run, name),
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
                git::file_diff(root, &entry_relative(root, path)?, staged)
            }
            WorkspaceBackend::Remote(remote) => remote.git_file_diff(path, staged),
        }
    }

    pub fn git_stage_file(&self, path: &str) -> Result<(), String> {
        match self {
            WorkspaceBackend::Local { root } => git::stage_file(root, &entry_relative(root, path)?),
            WorkspaceBackend::Remote(remote) => remote.git_stage_file(path),
        }
    }

    pub fn git_stage_all(&self) -> Result<(), String> {
        match self {
            WorkspaceBackend::Local { root } => git::stage_all(root),
            WorkspaceBackend::Remote(remote) => remote.git_stage_all(),
        }
    }

    pub fn git_initialize(&self) -> Result<(), String> {
        match self {
            WorkspaceBackend::Local { root } => git::initialize(root),
            WorkspaceBackend::Remote(remote) => remote.git_initialize(),
        }
    }

    pub fn git_unstage_file(&self, path: &str) -> Result<(), String> {
        match self {
            WorkspaceBackend::Local { root } => {
                git::unstage_file(root, &entry_relative(root, path)?)
            }
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

    pub fn read_file(&self, path: &str) -> Result<FileContents, String> {
        match self {
            WorkspaceBackend::Local { root } => {
                workspace_fs::read_file(root, &entry_relative(root, path)?)
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
                let relative = entry_relative(root, path)?;
                workspace_fs::write_file_checked(root, &relative, contents, expected_mtime_ms)
            }
            WorkspaceBackend::Remote(remote) => {
                remote.write_file(path, contents, expected_mtime_ms)
            }
        }
    }

    pub fn create_file(&self, path: &str) -> Result<FileStat, String> {
        match self {
            WorkspaceBackend::Local { root } => {
                workspace_fs::create_file(root, &entry_relative(root, path)?)
            }
            WorkspaceBackend::Remote(remote) => remote.create_file(path),
        }
    }

    pub fn create_directory(&self, path: &str) -> Result<FileStat, String> {
        match self {
            WorkspaceBackend::Local { root } => {
                dunlin_workspace::transfer::create_directory(root, &entry_relative(root, path)?)
            }
            WorkspaceBackend::Remote(remote) => remote.create_directory(path),
        }
    }

    pub fn move_entries(
        &self,
        sources: &[String],
        target: &str,
    ) -> Result<dunlin_workspace::transfer::TransferReport, String> {
        match self {
            WorkspaceBackend::Local { root } => {
                let sources = sources
                    .iter()
                    .map(|source| entry_relative(root, source))
                    .collect::<Result<Vec<_>, _>>()?;
                dunlin_workspace::transfer::move_entries(
                    root,
                    &sources,
                    &entry_relative(root, target)?,
                )
            }
            WorkspaceBackend::Remote(remote) => remote.move_entries(sources, target),
        }
    }

    pub fn stat_file(&self, path: &str) -> Result<FileStat, String> {
        match self {
            WorkspaceBackend::Local { root } => {
                workspace_fs::stat_file(root, &entry_relative(root, path)?)
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
}

fn branch_relay_error(error: String) -> String {
    if error.starts_with("Relay error 404:") || error.starts_with("Relay error 405:") {
        "Safe branch switching requires an updated Dunlin relay. No legacy checkout fallback was attempted.".into()
    } else {
        error
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

pub(crate) fn encode(value: &str) -> String {
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
    use super::{WorkspaceBackend, encode, validate_relay_url};
    use std::fs;

    #[test]
    fn relay_urls_require_tls_off_loopback() {
        assert!(validate_relay_url("https://relay.example.test").is_ok());
        assert!(validate_relay_url("http://127.0.0.1:8787").is_ok());
        assert!(validate_relay_url("http://localhost:8787").is_ok());
        assert!(validate_relay_url("http://192.168.1.10:8787").is_err());
        assert!(validate_relay_url("http://user:secret@relay.example.test").is_err());
        assert!(validate_relay_url("https://relay.example.test?token=x").is_err());
    }

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
    #[test]
    fn explorer_paths_are_confined_by_components_before_sending_them_to_a_relay() {
        assert_eq!(
            super::entry_relative("/project", "/project/src/file.rs").unwrap(),
            "src/file.rs"
        );
        assert_eq!(super::entry_relative("/project", "/project").unwrap(), "");
        assert_eq!(
            super::entry_relative("/project", "src/file.rs").unwrap(),
            "src/file.rs"
        );
        for path in [
            "/project-other/file",
            "/outside/file",
            "../outside",
            "src/../../outside",
            "/project/../outside",
        ] {
            assert!(super::entry_relative("/project", path).is_err(), "{path}");
        }
    }
}
