//! Read-only repository metadata and explicit imports through the existing gh account.
use serde::{Deserialize, Serialize};
use std::{process::Command, time::Duration};
pub mod imports;
pub(crate) mod process;
#[cfg(test)]
mod tests;
const LIMIT: u64 = 4 * 1024 * 1024;
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Metadata {
    pub full_name: String,
    pub language: Option<String>,
    pub topics: Vec<String>,
}
pub fn metadata(root: &str) -> Result<Metadata, String> {
    let identity = metadata_identity(root)?;
    let mut command = cli_command();
    command.current_dir(&identity.0).env("GH_REPO", format!("github.com/{}", identity.1))
        .args(["api", "--hostname", "github.com", "--method", "GET", "-H", "Accept: application/vnd.github+json", &format!("repos/{}", identity.1)]);
    let (status, bytes) = process::run(command, None, Duration::from_secs(20), 65536)
        .map_err(|_| "GitHub metadata unavailable: install/authenticate gh, or retry after the timeout/rate limit".to_string())?;
    if !status.success() { return Err("GitHub metadata unavailable: check gh authentication, repository access and rate limits".into()); }
    if metadata_identity(root)? != identity { return Err("Repository or GitHub remote changed while metadata loaded".into()); }
    let mut metadata: Metadata = serde_json::from_slice(&bytes).map_err(|_| "Invalid GitHub metadata response")?;
    if !metadata.full_name.eq_ignore_ascii_case(&identity.1)
        || metadata.topics.len() > 100
        || metadata.topics.iter().any(|topic| topic.is_empty() || topic.len() > 50 || !topic.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
        || metadata.language.as_ref().is_some_and(|s| s.len() > 100 || s.chars().any(char::is_control)) {
        return Err("Unsupported GitHub repository metadata".into());
    }
    metadata.topics.sort(); metadata.topics.dedup();
    Ok(metadata)
}
fn metadata_identity(root: &str) -> Result<(String, String), String> {
    let canonical = crate::git::history::repository_root(root, None)?;
    let link = crate::git::hosting::detect(&canonical).filter(|link| link.provider == crate::git::hosting::Provider::GitHub)
        .ok_or("No unambiguous GitHub remote")?;
    Ok((canonical, repository_name(&link.url)?))
}
fn component(value: &str) -> bool {
    !value.is_empty() && value.len() <= 100 && !matches!(value, "." | "..")
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
fn repository_name(remote: &str) -> Result<String, String> {
    crate::git::hosting::RepositoryLink::parse(remote)
        .filter(|link| link.provider == crate::git::hosting::Provider::GitHub)
        .map(|link| link.url.trim_start_matches("https://github.com/").to_string())
        .ok_or_else(|| "Only explicit GitHub.com HTTPS/SSH repository remotes are supported".into())
}
fn cli_command() -> Command {
    #[cfg(test)]
    let executable = tests::executable();
    #[cfg(not(test))]
    let executable = std::ffi::OsString::from("gh");
    let mut command = Command::new(executable);
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<_> = std::env::split_paths(&path).filter(|p| p.is_absolute()).collect();
    paths.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(std::path::PathBuf::from));
    if let Some(home) = std::env::var_os("HOME") { paths.push(std::path::PathBuf::from(home).join(".local/bin")); }
    if let Ok(path) = std::env::join_paths(paths) { command.env("PATH", path); }
    command.env("GH_HOST", "github.com").env_remove("GH_REPO").env("GH_PROMPT_DISABLED", "1")
        .env("GH_PAGER", "cat").env("PAGER", "cat").env("NO_COLOR", "1").env("CLICOLOR", "0")
        .env("GH_ACCESSIBLE_COLORS", "disabled").env_remove("GH_DEBUG").env_remove("GH_FORCE_TTY");
    command
}
