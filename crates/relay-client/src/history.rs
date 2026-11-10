//! Bounded history transport. Never falls back to a local path or an older API,
//! follows redirects, or exposes a token/raw server body in an error.
use super::{RemoteWorkspace, WorkspaceBackend, encode};
use dunlin_http as ureq;
use dunlin_workspace::git::history::{
    self, Commit, CommitDetails, CommitFile, CommitPatch, HistoryResponse,
};
use serde::de::DeserializeOwned;
use std::{
    io::Read,
    path::{Component, Path},
    time::Duration,
};

const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

fn relay_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(404 | 405, _) => "Git history requires an updated Dunlin relay. No local or legacy fallback was attempted.".into(),
        ureq::Error::Status(401 | 403, _) => "Relay Git history access was denied. Check pairing and the project's grant.".into(),
        ureq::Error::Status(409, _) => "The relay repository scope changed. Close and reopen Git history.".into(),
        ureq::Error::Status(429, _) => "Relay Git history is busy; try again.".into(),
        ureq::Error::Status(400, _) => "Relay Git history could not read this revision/comparison (invalid input, missing object or preview limit). No fallback was attempted.".into(),
        ureq::Error::Status(code, _) => format!("Relay Git history read failed (HTTP {code}); no fallback was attempted."),
        ureq::Error::Transport(_) => "Could not reach relay Git history (network error or timeout).".into(),
    }
}

fn get<T: DeserializeOwned>(
    remote: &RemoteWorkspace,
    path: &str,
    expected: Option<&str>,
) -> Result<HistoryResponse<T>, String> {
    super::validate_relay_url(&remote.host.url)?;
    let agent = super::http_agent();
    let response = agent
        .get(&format!(
            "{}{}",
            remote.host.url.trim_end_matches('/'),
            path
        ))
        .set("Authorization", &format!("Bearer {}", remote.token))
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(relay_error)?;
    if response.status() != 200 {
        return Err("Relay Git history refused an unexpected HTTP response/redirect".into());
    }
    if response
        .header("Content-Length")
        .and_then(|h| h.parse::<u64>().ok())
        .is_some_and(|n| n > MAX_RESPONSE_BYTES)
    {
        return Err("Relay Git history response exceeds the size limit".into());
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read relay Git history")?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err("Relay Git history response exceeds the size limit".into());
    }
    let response: HistoryResponse<T> =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid relay Git history response")?;
    validate_envelope(&response, expected)?;
    Ok(response)
}

fn validate_envelope<T>(
    response: &HistoryResponse<T>,
    expected: Option<&str>,
) -> Result<(), String> {
    if response.version != history::PROTOCOL_VERSION
        || response.scope.len() != 64
        || !response.scope.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Unsupported relay Git history protocol/scope. Update the relay; no fallback was attempted.".into());
    }
    if expected.is_some_and(|expected| expected != response.scope) {
        return Err("The relay repository scope changed. Close and reopen Git history.".into());
    }
    Ok(())
}

fn valid_oid(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64)
        && oid
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}
fn valid_commit(commit: &Commit) -> bool {
    valid_oid(&commit.oid)
        && commit.parents.len() < 2000
        && commit.parents.iter().all(|oid| valid_oid(oid))
        && !commit.author.contains('\0')
        && !commit.subject.contains('\0')
        && !commit.refs.contains('\0')
}
fn commit_bytes(commit: &Commit) -> usize {
    commit.oid.len()
        + commit.parents.iter().map(String::len).sum::<usize>()
        + commit.parents.len().saturating_sub(1)
        + commit.author.len()
        + commit.subject.len()
        + commit.refs.len()
        + commit.timestamp.to_string().len()
        + 6
}
fn valid_file(file: &CommitFile) -> bool {
    let path = |path: &str| {
        !path.is_empty()
            && !path.contains('\0')
            && !Path::new(path).is_absolute()
            && Path::new(path)
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
    };
    let status = file.status.as_bytes();
    status
        .first()
        .is_some_and(|s| matches!(s, b'A' | b'M' | b'D' | b'R' | b'C' | b'T'))
        && status.len() <= 8
        && status[1..].iter().all(u8::is_ascii_digit)
        && matches!(status[0], b'R' | b'C') == file.old_path.is_some()
        && path(&file.path)
        && file.old_path.as_deref().is_none_or(path)
}
fn file_bytes(file: &CommitFile) -> usize {
    file.status.len() + file.path.len() + 2 + file.old_path.as_ref().map_or(0, |old| old.len() + 1)
}

impl WorkspaceBackend {
    pub fn git_overview(
        &self,
        expected: Option<&str>,
    ) -> Result<HistoryResponse<dunlin_workspace::git::overview::Overview>, String> {
        use dunlin_workspace::git::overview;
        match self {
            Self::Local { root } => {
                let canonical = history::repository_root(root, expected)?;
                let value = overview::load(&canonical)?;
                history::repository_root(root, Some(&canonical))?;
                Ok(HistoryResponse {
                    version: history::PROTOCOL_VERSION,
                    scope: canonical,
                    value,
                })
            }
            Self::Remote(remote) => {
                let mut path = format!(
                    "/v1/git/overview?project={}",
                    encode(&remote.project.project_id)
                );
                if let Some(scope) = expected {
                    path.push_str(&format!("&scope={}", encode(scope)));
                }
                let response: HistoryResponse<overview::Overview> = get(remote, &path, expected)?;
                overview::validate(&response.value)?;
                Ok(response)
            }
        }
    }

    pub fn history_load(
        &self,
        revision: &str,
        limit: usize,
        expected: Option<&str>,
    ) -> Result<HistoryResponse<Vec<Commit>>, String> {
        match self {
            Self::Local { root } => {
                let canonical = history::repository_root(root, expected)?;
                let value = history::load_from(&canonical, revision, limit)?;
                history::repository_root(root, Some(&canonical))?;
                Ok(HistoryResponse {
                    version: history::PROTOCOL_VERSION,
                    scope: canonical,
                    value,
                })
            }
            Self::Remote(remote) => {
                let mut path = format!(
                    "/v1/git/history?project={}&revision={}&limit={}",
                    encode(&remote.project.project_id),
                    encode(revision),
                    limit.clamp(1, 2000)
                );
                if let Some(scope) = expected {
                    path.push_str(&format!("&scope={}", encode(scope)));
                }
                let response: HistoryResponse<Vec<Commit>> = get(remote, &path, expected)?;
                if response.value.len() > limit.clamp(1, 2000)
                    || !response.value.iter().all(valid_commit)
                    || response.value.iter().map(commit_bytes).sum::<usize>() > 2 * 1024 * 1024
                {
                    return Err("Invalid or oversized relay commit list".into());
                }
                Ok(response)
            }
        }
    }

    pub fn history_details(
        &self,
        scope: &str,
        oid: &str,
        parent: usize,
    ) -> Result<CommitDetails, String> {
        match self {
            Self::Local { root } => {
                let canonical = history::repository_root(root, Some(scope))?;
                let detail = history::details(&canonical, oid, parent)?;
                history::repository_root(root, Some(&canonical))?;
                Ok(detail)
            }
            Self::Remote(remote) => {
                let path = format!(
                    "/v1/git/history/commit?project={}&scope={}&oid={}&parent={parent}",
                    encode(&remote.project.project_id),
                    encode(scope),
                    encode(oid)
                );
                let response: HistoryResponse<CommitDetails> = get(remote, &path, Some(scope))?;
                let detail = response.value;
                let base = if detail.commit.parents.is_empty() && parent == 0 {
                    Some(None)
                } else {
                    detail
                        .commit
                        .parents
                        .get(parent)
                        .map(|id| Some(id.as_str()))
                };
                if detail.commit.oid != oid
                    || !valid_commit(&detail.commit)
                    || base != Some(detail.parent.as_deref())
                    || detail.message.len() > 64 * 1024
                    || commit_bytes(&detail.commit) + detail.message.len() > 64 * 1024
                    || detail.files.len() > 2000
                    || !detail.files.iter().all(valid_file)
                    || detail.files.iter().map(file_bytes).sum::<usize>() > 512 * 1024
                {
                    return Err("Invalid relay commit comparison".into());
                }
                Ok(detail)
            }
        }
    }

    pub fn history_patch(
        &self,
        scope: &str,
        oid: &str,
        parent: usize,
        index: usize,
        file: &CommitFile,
    ) -> Result<String, String> {
        match self {
            Self::Local { root } => {
                let canonical = history::repository_root(root, Some(scope))?;
                let patch = history::file_patch(&canonical, oid, parent, file)?;
                history::repository_root(root, Some(&canonical))?;
                Ok(patch)
            }
            Self::Remote(remote) => {
                let path = format!(
                    "/v1/git/history/patch?project={}&scope={}&oid={}&parent={parent}&file={index}",
                    encode(&remote.project.project_id),
                    encode(scope),
                    encode(oid)
                );
                let response: HistoryResponse<CommitPatch> = get(remote, &path, Some(scope))?;
                let value = response.value;
                if value.oid != oid
                    || value.parent != parent
                    || value.file != *file
                    || value.patch.len() > 2 * 1024 * 1024
                    || value.patch.lines().count() > 20_000
                {
                    return Err("Invalid, mismatched or oversized relay commit patch".into());
                }
                Ok(value.patch)
            }
        }
    }
}

#[cfg(test)]
mod tests;
