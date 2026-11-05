//! Explicit previews and non-overwriting clones; credentials stay inside gh.
use super::{cli_command, component, process, repository_name};
use serde::Deserialize;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::Duration,
};

pub const MAX_RESULTS: usize = 1000;

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

fn api<T: serde::de::DeserializeOwned>(endpoint: &str, fields: &[String]) -> Result<T, String> {
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
    let (status, bytes) = process::run(command, None, Duration::from_secs(30), super::LIMIT)?;
    if !status.success() {
        return Err("GitHub request failed. Check the query, gh auth login, repository permissions and rate limits.".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid GitHub repository response".into())
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
fn valid_name(name: &str) -> bool {
    let parts: Vec<_> = name.split('/').collect();
    parts.len() == 2 && parts.iter().all(|part| component(part))
}

pub fn preview(source: &Source) -> Result<Vec<String>, String> {
    match source {
        Source::Repository(name) => {
            if !valid_name(name) {
                return Err("Invalid GitHub repository name".into());
            }
            let repo: Repo = api(&format!("repos/{name}"), &[])?;
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
                let result: Page = api(
                    "search/repositories",
                    &[
                        format!("q={query}"),
                        "per_page=100".into(),
                        format!("page={page}"),
                    ],
                )?;
                if result.incomplete_results {
                    return Err(
                        "GitHub returned incomplete results. Narrow the filter and preview again."
                            .into(),
                    );
                }
                if result.total_count > MAX_RESULTS {
                    return Err(format!(
                        "{} repositories match. Narrow the filter to at most {MAX_RESULTS} repositories.",
                        result.total_count
                    ));
                }
                if expected.is_some_and(|count| count != result.total_count) {
                    return Err("Search results changed. Preview again before importing.".into());
                }
                expected = Some(result.total_count);
                if result.items.len() > 100 {
                    return Err("Oversized GitHub search page".into());
                }
                for repo in result.items {
                    if !valid_name(&repo.full_name) || !seen.insert(repo.full_name.to_lowercase()) {
                        return Err(
                            "Invalid or changing GitHub search results. Preview again.".into()
                        );
                    }
                    repos.push(repo.full_name);
                }
                if repos.len() == result.total_count {
                    return Ok(repos);
                }
                if repos.len() > result.total_count || repos.len() < page * 100 {
                    return Err("Incomplete GitHub search results. Preview again.".into());
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
    if !destination.is_absolute() {
        return Err("Choose an absolute destination folder".into());
    }
    std::fs::create_dir_all(destination).map_err(|e| format!("Cannot create destination: {e}"))?;
    let destination = std::fs::canonicalize(destination).map_err(|e| e.to_string())?;
    let (owner, repo) = name.split_once('/').unwrap();
    let parent = destination.join(owner);
    match std::fs::create_dir(&parent) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("Cannot create owner folder: {e}")),
    }
    if !std::fs::symlink_metadata(&parent).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
    {
        return Err("The owner folder must be a real directory, not a symlink".into());
    }
    let path = parent.join(repo);
    // Reserve the name atomically. Never reuse or delete an existing checkout.
    std::fs::create_dir(&path).map_err(|e| {
        format!(
            "Cannot clone {name} into {} (existing folders are never overwritten): {e}",
            path.display()
        )
    })?;
    let mut command = cli_command();
    command
        .current_dir(&destination)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["repo", "clone", &format!("https://github.com/{name}")])
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
            "Clone failed for {name}. Any partial files remain at {}; inspect them before retrying. Check gh authentication, Git credentials and disk space.",
            path.display()
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
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
