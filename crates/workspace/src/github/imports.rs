//! Explicit previews and non-overwriting clones; credentials stay inside gh.
use super::{cli_command, component, process, repository_name};
use serde::Deserialize;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub const MAX_RESULTS: usize = 1000;

pub struct PreviewRequest {
    pub(crate) cancelled: AtomicBool,
    started: Instant,
}
impl Default for PreviewRequest {
    fn default() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            started: Instant::now(),
        }
    }
}
impl PreviewRequest {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub(crate) fn remaining(&self) -> Result<Duration, String> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err("Search cancelled".into());
        }
        Duration::from_secs(30)
            .checked_sub(self.started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| "Repository search timed out. Narrow the query and try again.".into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Repository(String),
    Search(String),
}
impl Source {
    pub fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();
        if input.is_empty() || input.len() > 4096 || input.chars().any(char::is_control) {
            return Err("Enter a GitHub repository URL or repository search query".into());
        }
        if input.starts_with("https://") || input.starts_with("git@") || input.starts_with("ssh://")
        {
            return repository_name(input.trim_end_matches('/')).map(Self::Repository);
        }
        if input.contains("://") || input.starts_with("github.com/") {
            return Err("Use an explicit https://github.com/owner/repo URL".into());
        }
        let parts: Vec<_> = input.split('/').collect();
        if parts.len() == 2 && parts.iter().all(|part| component(part)) {
            return repository_name(&format!("https://github.com/{input}")).map(Self::Repository);
        }
        Ok(Self::Search(input.into()))
    }
}

#[derive(Clone, Debug)]
pub struct CliStatus {
    pub installed: bool,
    pub authenticated: bool,
    pub message: String,
}
pub fn status() -> CliStatus {
    let mut version = cli_command();
    version.arg("--version");
    let installed = process::run(version, None, Duration::from_secs(5), 65536)
        .is_ok_and(|(status, _)| status.success());
    let authenticated = installed && {
        let mut auth = cli_command();
        auth.args(["auth", "status", "--hostname", "github.com", "--active"]);
        process::run(auth, None, Duration::from_secs(15), 65536)
            .is_ok_and(|(status, _)| status.success())
    };
    CliStatus {
        installed,
        authenticated,
        message: if !installed {
            "Not installed · install GitHub CLI (gh)"
        } else if !authenticated {
            "Installed · not authorized · run gh auth login"
        } else {
            "Installed · authorized for github.com"
        }
        .into(),
    }
}

pub(crate) fn api<T: serde::de::DeserializeOwned>(
    endpoint: &str,
    fields: &[String],
) -> Result<T, String> {
    let bytes = request(endpoint, fields, false, &PreviewRequest::default())?;
    serde_json::from_slice(&bytes).map_err(|_| "Invalid GitHub repository response".into())
}

fn request(
    endpoint: &str,
    fields: &[String],
    include_headers: bool,
    search: &PreviewRequest,
) -> Result<Vec<u8>, String> {
    let mut command = cli_command();
    command.args([
        "api",
        "--hostname",
        "github.com",
        "--method",
        "GET",
        "-H",
        "Accept: application/vnd.github+json",
        endpoint,
    ]);
    for field in fields {
        command.arg("-f").arg(field);
    }
    if include_headers {
        command.arg("--include");
    }
    let (status, bytes) = process::run_cancellable(command, None, search.remaining()?, super::LIMIT, &search.cancelled)
        .map_err(|_| "GitHub repository lookup unavailable, cancelled or timed out. Check connectivity or narrow the query.".to_string())?;
    if !status.success() {
        return Err("GitHub request failed. Check the query, gh auth login, repository permissions and rate limits.".into());
    }
    Ok(bytes)
}
#[derive(Deserialize)]
struct Repo {
    full_name: String,
}
#[derive(Deserialize)]
struct Page {
    total_count: usize,
    incomplete_results: bool,
    items: Vec<Repo>,
}

fn search_page(query: &str, page: usize, search: &PreviewRequest) -> Result<(Page, bool), String> {
    let bytes = request(
        "search/repositories",
        &[
            format!("q={query}"),
            "per_page=100".into(),
            format!("page={page}"),
        ],
        true,
        search,
    )?;
    let (end, separator) = bytes
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .map(|end| (end, 4))
        .or_else(|| {
            bytes
                .windows(2)
                .position(|part| part == b"\n\n")
                .map(|end| (end, 2))
        })
        .ok_or("Missing GitHub response headers")?;
    let headers =
        std::str::from_utf8(&bytes[..end]).map_err(|_| "Invalid GitHub response headers")?;
    if !headers.starts_with("HTTP/") {
        return Err("Invalid GitHub response headers".into());
    }
    let next = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("link"))
        .flat_map(|(_, value)| value.split(','))
        .any(|link| {
            link.split(';')
                .skip(1)
                .any(|part| part.trim() == "rel=\"next\"")
        });
    let result = serde_json::from_slice(&bytes[end + separator..])
        .map_err(|_| "Invalid GitHub repository response")?;
    Ok((result, next))
}
fn valid_name(name: &str) -> bool {
    let parts: Vec<_> = name.split('/').collect();
    parts.len() == 2 && parts.iter().all(|part| component(part))
}

pub fn preview(source: &Source) -> Result<Vec<String>, String> {
    preview_cancellable(source, &PreviewRequest::default())
}

pub fn preview_cancellable(
    source: &Source,
    search: &PreviewRequest,
) -> Result<Vec<String>, String> {
    match source {
        Source::Repository(name) => {
            if !valid_name(name) {
                return Err("Invalid GitHub repository name".into());
            }
            let repo: Repo =
                serde_json::from_slice(&request(&format!("repos/{name}"), &[], false, search)?)
                    .map_err(|_| "Invalid GitHub repository response")?;
            if !valid_name(&repo.full_name) {
                return Err("Invalid GitHub repository name".into());
            }
            Ok(vec![repo.full_name])
        }
        Source::Search(query) => {
            let mut repos = Vec::new();
            let mut seen = HashSet::new();
            let mut expected = None;
            for page in 1..=10 {
                let (result, next) = search_page(query, page, search)?;
                if result.incomplete_results {
                    return Err(
                        "GitHub returned incomplete results. Narrow the filter and preview again."
                            .into(),
                    );
                }
                if expected.is_some_and(|count| count != result.total_count) {
                    return Err("Search results changed. Preview again before importing.".into());
                }
                expected = Some(result.total_count);
                if result.items.len() > 100 {
                    return Err("Oversized GitHub search page".into());
                }
                if next && result.items.is_empty() {
                    return Err("Incomplete GitHub search results. Preview again.".into());
                }
                for repo in result.items {
                    if !valid_name(&repo.full_name) || !seen.insert(repo.full_name.to_lowercase()) {
                        return Err(
                            "Invalid or changing GitHub search results. Preview again.".into()
                        );
                    }
                    repos.push(repo.full_name);
                }
                // Match gh: Link pagination, not total_count, determines the actual result set.
                if !next {
                    return Ok(repos);
                }
                if page == MAX_RESULTS / 100 {
                    return Err(format!(
                        "Search exceeds the 10-page limit (up to {MAX_RESULTS} repositories). Narrow the filter and search again."
                    ));
                }
            }
            Err("Incomplete GitHub search results. Narrow the filter.".into())
        }
    }
}

pub fn clone_repository(name: &str, destination: &Path) -> Result<PathBuf, String> {
    if !valid_name(name) {
        return Err("Invalid GitHub repository name".into());
    }
    clone_with(cli_command(), "github.com", name, destination)
}

pub fn clone_with(
    mut command: std::process::Command,
    host: &str,
    name: &str,
    destination: &Path,
) -> Result<PathBuf, String> {
    if !matches!(host, "github.com" | "gitlab.com")
        || name.split('/').count() < 2
        || !name.split('/').all(component)
    {
        return Err("Invalid hosting repository path".into());
    }
    if !destination.is_absolute() {
        return Err("Choose an absolute destination folder".into());
    }
    std::fs::create_dir_all(destination).map_err(|e| format!("Cannot create destination: {e}"))?;
    let destination = std::fs::canonicalize(destination).map_err(|e| e.to_string())?;
    let (namespace, repo) = name.rsplit_once('/').unwrap();
    let mut parent = destination.clone();
    for part in namespace.split('/') {
        parent.push(part);
        match std::fs::create_dir(&parent) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("Cannot create owner folder: {e}")),
        }
        if !std::fs::symlink_metadata(&parent)
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        {
            return Err("The owner folder must be a real directory, not a symlink".into());
        }
    }
    let path = parent.join(repo);
    // Reserve the name atomically. Never reuse or delete an existing checkout.
    std::fs::create_dir(&path).map_err(|e| {
        format!(
            "Cannot clone {name} into {} (existing folders are never overwritten): {e}",
            path.display()
        )
    })?;
    crate::git::command::scrub(&mut command);
    command
        .current_dir(&destination)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["repo", "clone", &format!("https://{host}/{name}")])
        .arg(&path)
        .args([
            "--",
            "--template=",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
        ]);
    let result = process::run(command, None, Duration::from_secs(600), super::LIMIT);
    if !result.is_ok_and(|(status, _)| status.success()) || !crate::git::is_repository_root(&path) {
        return Err(format!(
            "Clone failed for {name}. Any partial files remain at {}; inspect them before retrying. Check hosting CLI authentication, Git credentials and disk space.",
            path.display()
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_deadline_is_shared_across_pages_and_cancellation_is_immediate() {
        let request = PreviewRequest::default();
        assert!(request.remaining().unwrap() <= Duration::from_secs(30));
        request.cancel();
        assert_eq!(request.remaining().unwrap_err(), "Search cancelled");
        let expired = PreviewRequest {
            cancelled: AtomicBool::new(false),
            started: Instant::now() - Duration::from_secs(31),
        };
        assert!(expired.remaining().unwrap_err().contains("timed out"));
    }
    #[test]
    fn parses_urls_without_inventing_a_filter_language() {
        for value in [
            "https://github.com/owner/repo.git/",
            "git@github.com:owner/repo.git",
            "owner/repo",
        ] {
            assert_eq!(
                Source::parse(value).unwrap(),
                Source::Repository("owner/repo".into())
            );
        }
        let query = "org:example archived:false language:Rust stars:>=10";
        assert_eq!(Source::parse(query).unwrap(), Source::Search(query.into()));
        for value in [
            "",
            "https://token@github.com/o/r",
            "https://github.com/o/r/tree/main",
            "https://evil.com/o/r",
            "a\nb",
        ] {
            assert!(Source::parse(value).is_err(), "{value}");
        }
    }
    #[test]
    fn refuses_existing_folders_and_symlinked_owners_without_touching_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owner/repo");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("keep"), "keep").unwrap();
        assert!(clone_repository("owner/repo", dir.path()).is_err());
        assert_eq!(std::fs::read_to_string(path.join("keep")).unwrap(), "keep");
        assert!(clone_repository("../escape", dir.path()).is_err());
        assert!(clone_repository("owner/repo", Path::new("relative")).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path(), dir.path().join("linked")).unwrap();
            assert!(clone_repository("linked/new", dir.path()).is_err());
            assert!(!dir.path().join("new").exists());
        }
    }
}
