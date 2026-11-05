//! Hosting links from explicit Git remotes only; no credentials or network calls.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider { GitHub, GitLab }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryLink { pub provider: Provider, pub url: String }
impl RepositoryLink {
    pub fn parse(remote: &str) -> Option<Self> {
        if remote.len() > 2048 { return None; }
        for (host, provider) in [("github.com", Provider::GitHub), ("gitlab.com", Provider::GitLab)] {
            let path = [format!("https://{host}/"), format!("git@{host}:"), format!("ssh://git@{host}/")]
                .iter().find_map(|prefix| remote.strip_prefix(prefix.as_str()));
            if let Some(path) = path {
                let path = path.trim_end_matches('/');
                let path = path.strip_suffix(".git").unwrap_or(path);
                let parts: Vec<_> = path.split('/').collect();
                if parts.len() < 2 || (provider == Provider::GitHub && parts.len() != 2)
                    || !parts.iter().all(|part| !part.is_empty() && part.len() <= 100 && !matches!(*part, "." | "..")
                        && part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))) { return None; }
                return Some(Self { provider, url: format!("https://{host}/{path}") });
            }
        }
        None
    }
    pub fn validated(&self) -> bool { Self::parse(&self.url).as_ref() == Some(self) }
}
pub(crate) fn detect(root: &str) -> Option<RepositoryLink> {
    let remotes = super::activity::git_limited(root, &["remote"], 65536).ok()?;
    let names: Vec<_> = remotes.lines().collect();
    if names.len() > 32 { return None; }
    let identify = |name: &str| {
        if name.starts_with('-') || name.len() > 100 || !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) { return None; }
        let urls = super::activity::git_limited(root, &["remote", "get-url", "--all", name], 65536).ok()?;
        if urls.lines().count() != 1 { return None; }
        RepositoryLink::parse(urls.trim_end_matches('\n'))
    };
    if names.contains(&"origin") { return identify("origin"); }
    let mut links: Vec<_> = names.into_iter().filter_map(identify).collect();
    links.dedup();
    (links.len() == 1).then(|| links.remove(0))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_public_links_and_nested_gitlab_paths_only() {
        for remote in ["git@github.com:owner/repo.git", "ssh://git@github.com/owner/repo", "https://github.com/owner/repo.git/"] {
            let link = RepositoryLink::parse(remote).unwrap();
            assert_eq!(link.url, "https://github.com/owner/repo"); assert!(link.validated());
        }
        assert_eq!(RepositoryLink::parse("git@gitlab.com:group/subgroup/repo.git").unwrap().url, "https://gitlab.com/group/subgroup/repo");
        for remote in ["https://token@github.com/o/r", "https://github.com.evil/o/r", "https://gitlab.example/o/r", "https://gitlab.com/a/../b", "https://github.com/o/r?q=x", "file:///tmp/repo", "https://github.com/o/r\n", "https://github.com/o/r/extra"] {
            assert!(RepositoryLink::parse(remote).is_none(), "{remote}");
        }
        assert!(!RepositoryLink { provider: Provider::GitHub, url: "https://gitlab.com/g/r".into() }.validated());
        let old: super::super::GitStatus = serde_json::from_str(r#"{"branch":"main","changes":[],"error":null}"#).unwrap();
        assert!(old.repository.is_none());
    }
    #[test]
    fn configured_remotes_update_links_without_network_or_a_head_commit() {
        let root = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(root.path()).args(args).status().unwrap().success());
        git(&["init", "--template=", "-q"]);
        git(&["remote", "add", "origin", "git@github.com:o/r.git"]);
        assert_eq!(super::super::load(root.path().to_str().unwrap()).repository.unwrap().provider, Provider::GitHub);
        git(&["remote", "set-url", "origin", "git@gitlab.com:g/sub/r.git"]);
        assert_eq!(detect(root.path().to_str().unwrap()).unwrap().provider, Provider::GitLab);
        git(&["remote", "set-url", "origin", "https://example.com/o/r.git"]);
        assert!(detect(root.path().to_str().unwrap()).is_none());
        git(&["remote", "remove", "origin"]);
        git(&["remote", "add", "upstream", "https://github.com/o/r"]);
        assert!(detect(root.path().to_str().unwrap()).is_some());
        git(&["remote", "add", "other", "https://github.com/o/other"]);
        assert!(detect(root.path().to_str().unwrap()).is_none());
    }
}
