//! GitLab.com imports through glab; credentials never leave the CLI.
use crate::{
    git::hosting::{Provider, RepositoryLink},
    github::{
        imports::{CliStatus, MAX_RESULTS, PreviewRequest, Source},
        process,
    },
};
use serde::Deserialize;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
#[cfg(test)]
thread_local! { static EXECUTABLE: std::cell::RefCell<std::ffi::OsString> = std::cell::RefCell::new("glab".into()); }

pub(crate) fn command() -> Command {
    let command = crate::github::cli_command();
    // Reuse the bounded process environment and GUI PATH, not gh's executable.
    #[cfg(test)]
    let executable = EXECUTABLE.with(|value| value.borrow().clone());
    #[cfg(not(test))]
    let executable = std::ffi::OsString::from("glab");
    let mut glab = Command::new(executable);
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            glab.env(key, value);
        } else {
            glab.env_remove(key);
        }
    }
    glab.env("GITLAB_HOST", "gitlab.com")
        .env("GITLAB_URI", "https://gitlab.com")
        .env_remove("GITLAB_URL")
        .env_remove("GLAB_DEBUG_HTTP")
        .env("GLAB_PROMPT_DISABLED", "1");
    glab
}

pub fn status() -> CliStatus {
    let mut version = command();
    version.arg("--version");
    let installed =
        process::run(version, None, Duration::from_secs(5), 65536).is_ok_and(|(s, _)| s.success());
    let authenticated = installed && {
        let mut auth = command();
        auth.args(["auth", "status", "--hostname", "gitlab.com"]);
        process::run(auth, None, Duration::from_secs(15), 65536).is_ok_and(|(s, _)| s.success())
    };
    CliStatus {
        installed,
        authenticated,
        message: if !installed {
            "Not installed · install GitLab CLI (glab)"
        } else if !authenticated {
            "Installed · not authorized · run glab auth login --hostname gitlab.com"
        } else {
            "Installed · authorized for gitlab.com"
        }
        .into(),
    }
}

pub(crate) fn api<T: serde::de::DeserializeOwned>(
    endpoint: &str,
    fields: &[String],
) -> Result<T, String> {
    api_cancellable(endpoint, fields, &PreviewRequest::default())
}

fn api_cancellable<T: serde::de::DeserializeOwned>(
    endpoint: &str,
    fields: &[String],
    search: &PreviewRequest,
) -> Result<T, String> {
    let mut cmd = command();
    cmd.args([
        "api",
        "--hostname",
        "gitlab.com",
        "--method",
        "GET",
        endpoint,
    ]);
    for field in fields {
        cmd.arg("--raw-field").arg(field);
    }
    let (status, bytes) = process::run_cancellable(
        cmd,
        None,
        search.remaining()?,
        4 * 1024 * 1024,
        &search.cancelled,
    )
    .map_err(|_| "GitLab CLI unavailable, cancelled or timed out".to_string())?;
    if !status.success() {
        return Err(
            "GitLab request failed. Check glab authentication, access and rate limits.".into(),
        );
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid GitLab response".into())
}

fn valid_path(path: &str) -> bool {
    RepositoryLink::parse(&format!("https://gitlab.com/{path}"))
        .is_some_and(|link| link.url == format!("https://gitlab.com/{path}"))
}
pub fn parse(input: &str) -> Result<Source, String> {
    let input = input.trim();
    if input.is_empty() || input.len() > 4096 || input.chars().any(char::is_control) {
        return Err("Enter a GitLab.com repository URL or search text".into());
    }
    if input.starts_with("https://") || input.starts_with("git@") || input.starts_with("ssh://") {
        return RepositoryLink::parse(input)
            .filter(|link| link.provider == Provider::GitLab)
            .map(|link| {
                Source::Repository(link.url.trim_start_matches("https://gitlab.com/").into())
            })
            .ok_or_else(|| "Use an explicit GitLab.com repository URL".into());
    }
    if input.contains("://") || input.starts_with("gitlab.com/") {
        return Err("Use https://gitlab.com/group/project".into());
    }
    if input.contains('/') {
        return if valid_path(input) {
            Ok(Source::Repository(input.into()))
        } else {
            Err("Invalid GitLab project path".into())
        };
    }
    Ok(Source::Search(input.into()))
}
#[derive(Deserialize)]
struct Project {
    path_with_namespace: String,
}
pub fn preview(source: &Source) -> Result<Vec<String>, String> {
    preview_cancellable(source, &PreviewRequest::default())
}

pub fn preview_cancellable(
    source: &Source,
    search: &PreviewRequest,
) -> Result<Vec<String>, String> {
    match source {
        Source::Repository(path) => {
            if !valid_path(path) {
                return Err("Invalid GitLab project path".into());
            }
            let project: Project = api_cancellable(
                &format!("projects/{}", path.replace('/', "%2F")),
                &[],
                search,
            )?;
            if !project.path_with_namespace.eq_ignore_ascii_case(path)
                || !valid_path(&project.path_with_namespace)
            {
                return Err("GitLab project identity changed. Preview again.".into());
            }
            Ok(vec![project.path_with_namespace])
        }
        Source::Search(query) => {
            let mut names = Vec::new();
            let mut seen = HashSet::new();
            for page in 1..=11 {
                let projects: Vec<Project> = api_cancellable(
                    "projects",
                    &[
                        "membership=true".into(),
                        format!("search={query}"),
                        "order_by=id".into(),
                        "sort=asc".into(),
                        "per_page=100".into(),
                        format!("page={page}"),
                    ],
                    search,
                )?;
                let count = projects.len();
                if count > 100 {
                    return Err("Oversized GitLab page".into());
                }
                for project in projects {
                    if !valid_path(&project.path_with_namespace)
                        || !seen.insert(project.path_with_namespace.to_lowercase())
                    {
                        return Err("Invalid or changing GitLab results. Preview again.".into());
                    }
                    names.push(project.path_with_namespace);
                }
                if names.len() > MAX_RESULTS {
                    return Err(
                        "Too many GitLab projects. Narrow the search to at most 1000.".into(),
                    );
                }
                if count < 100 {
                    return Ok(names);
                }
            }
            Err("Incomplete GitLab results. Narrow the search.".into())
        }
    }
}
pub fn clone_repository(name: &str, destination: &Path) -> Result<PathBuf, String> {
    if !valid_path(name) {
        return Err("Invalid GitLab project path".into());
    }
    crate::github::imports::clone_with(command(), "gitlab.com", name, destination)
}

pub fn metadata(root: &str) -> Result<crate::github::Metadata, String> {
    let root = crate::git::history::repository_root(root, None)?;
    let link = crate::git::hosting::detect(&root)
        .filter(|link| link.provider == Provider::GitLab)
        .ok_or("No unambiguous GitLab remote")?;
    let path = link.url.trim_start_matches("https://gitlab.com/");
    #[derive(Deserialize)]
    struct Details {
        path_with_namespace: String,
        #[serde(default)]
        topics: Vec<String>,
    }
    let details: Details = api(&format!("projects/{}", path.replace('/', "%2F")), &[])?;
    if !details.path_with_namespace.eq_ignore_ascii_case(path)
        || details.topics.len() > 100
        || details
            .topics
            .iter()
            .any(|s| s.is_empty() || s.len() > 100 || s.chars().any(char::is_control))
        || crate::git::hosting::detect(&root).as_ref() != Some(&link)
    {
        return Err("Unsupported or changed GitLab project metadata".into());
    }
    let mut topics = details.topics;
    topics.sort();
    topics.dedup();
    Ok(crate::github::Metadata {
        full_name: details.path_with_namespace,
        topics,
        language: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_paths_are_exact_and_never_accept_other_hosts_or_traversal() {
        for value in [
            "group/sub/repo",
            "https://gitlab.com/group/sub/repo.git",
            "git@gitlab.com:group/sub/repo.git",
        ] {
            assert_eq!(
                parse(value).unwrap(),
                Source::Repository("group/sub/repo".into())
            );
        }
        assert_eq!(
            parse("encoding service").unwrap(),
            Source::Search("encoding service".into())
        );
        for value in [
            "",
            "https://github.com/o/r",
            "https://token@gitlab.com/g/r",
            "https://gitlab.example/g/r",
            "group/../repo",
            "g/r?x=1",
            "g/r\nunsafe",
        ] {
            assert!(parse(value).is_err(), "{value}");
        }
    }
    #[test]
    fn clones_never_reuse_a_checkout_or_follow_a_nested_namespace_symlink() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("group/sub/repo")).unwrap();
        assert!(clone_repository("group/sub/repo", dir.path()).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path(), dir.path().join("group/link")).unwrap();
            assert!(clone_repository("group/link/new", dir.path()).is_err());
            assert!(!dir.path().join("new").exists());
        }
    }

    struct Fixture(tempfile::TempDir);
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let script = dir.path().join("glab");
            std::fs::write(
                &script,
                r##"#!/usr/bin/env python3
import json, sys, pathlib, subprocess
root = pathlib.Path(__file__).parent
a = sys.argv[1:]
with (root/'calls').open('a') as f: f.write(json.dumps(a)+'\n')
if a[:2] == ['repo', 'clone']:
 subprocess.run(['git', 'init', '--template=', '-q', a[3]], check=True)
 sys.exit(0)
endpoint = a[5] if a[:1] == ['api'] else ('version' if a == ['--version'] else 'auth')
page = next((s for s in a if s.startswith('page=')), '')
data = json.loads((root/'responses').read_text()).get(endpoint + page)
if data is None:
 print('private token must not escape', file=sys.stderr); sys.exit(1)
print(json.dumps(data))
"##,
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            EXECUTABLE.with(|value| *value.borrow_mut() = script.into_os_string());
            Self(dir)
        }
        fn responses(&self, value: serde_json::Value) {
            std::fs::write(self.0.path().join("responses"), value.to_string()).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            EXECUTABLE.with(|value| *value.borrow_mut() = "glab".into());
        }
    }
    #[test]
    fn exact_project_profile_status_and_clone_use_only_the_explicit_gitlab_account() {
        let f = Fixture::new();
        f.responses(serde_json::json!({"version":{}, "auth":{}, "projects/group%2Fsub%2Frepo":{"path_with_namespace":"group/sub/repo"}, "user":{"id":42,"username":"fixture","name":"Fixture User","avatar_url":null}}));
        assert!(status().authenticated);
        assert_eq!(
            preview(&Source::Repository("group/sub/repo".into())).unwrap(),
            ["group/sub/repo"]
        );
        assert_eq!(
            crate::hosting_account::load(Provider::GitLab).unwrap().name,
            "Fixture User"
        );
        let destination = f.0.path().join("clones");
        assert!(
            clone_repository("group/sub/repo", &destination)
                .unwrap()
                .join(".git")
                .exists()
        );
        assert!(clone_repository("group/sub/repo", &destination).is_err());
        let calls = std::fs::read_to_string(f.0.path().join("calls")).unwrap();
        assert!(
            calls.contains("https://gitlab.com/group/sub/repo")
                && calls.contains("core.hooksPath=/dev/null")
        );
        assert!(calls.contains("--hostname\", \"gitlab.com\", \"--method\", \"GET"));
    }
    #[test]
    fn search_paginates_and_rejects_duplicates_wrong_identity_and_secret_errors() {
        let f = Fixture::new();
        let projects: Vec<_> = (0..100)
            .map(|i| serde_json::json!({"path_with_namespace":format!("g/sub/r{i}")}))
            .collect();
        f.responses(serde_json::json!({"projectspage=1":projects, "projectspage=2":[{"path_with_namespace":"g/sub/last"}]}));
        assert_eq!(
            preview(&Source::Search("service".into())).unwrap().len(),
            101
        );
        f.responses(serde_json::json!({"projectspage=1":[{"path_with_namespace":"g/r"},{"path_with_namespace":"g/r"}]}));
        assert!(preview(&Source::Search("service".into())).is_err());
        f.responses(serde_json::json!({"projects/g%2Fr":{"path_with_namespace":"other/repo"}}));
        assert!(preview(&Source::Repository("g/r".into())).is_err());
        f.responses(serde_json::json!({}));
        assert!(
            !preview(&Source::Search("service".into()))
                .unwrap_err()
                .contains("private token")
        );
    }

    #[test]
    fn metadata_requires_exact_nested_repository_identity_and_safe_topics() {
        let f = Fixture::new();
        let root = f.0.path().join("repo");
        for args in [
            vec!["init", "--template=", "-q"],
            vec!["remote", "add", "origin", "https://gitlab.com/g/sub/r"],
        ] {
            std::fs::create_dir_all(&root).unwrap();
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(&root)
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        f.responses(serde_json::json!({"projects/g%2Fsub%2Fr":{"path_with_namespace":"g/sub/r", "topics":["rust", "cloud", "rust"]}}));
        assert_eq!(
            metadata(root.to_str().unwrap()).unwrap().topics,
            ["cloud", "rust"]
        );
        for data in [
            serde_json::json!({"path_with_namespace":"other/repo", "topics":[]}),
            serde_json::json!({"path_with_namespace":"g/sub/r", "topics":["bad\nlabel"]}),
        ] {
            f.responses(serde_json::json!({"projects/g%2Fsub%2Fr":data}));
            assert!(metadata(root.to_str().unwrap()).is_err());
        }
    }
}
