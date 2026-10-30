//! Bounded, read-only project discovery. A collection is not itself a workspace
//! scope. Repositories stop traversal; symlinked children never widen selection.
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const MAX_DEPTH: usize = 8;
pub const MAX_ENTRIES: usize = 20_000;
pub const MAX_REPOSITORIES: usize = 256;
const MAX_MANIFEST: u64 = 256 * 1024;
#[derive(Clone, Debug)]
pub struct LocalProject {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub monorepo: Option<String>,
}
#[derive(Default, Debug)]
pub struct Discovery {
    pub projects: Vec<LocalProject>,
    pub warnings: Vec<String>,
    pub limited: bool,
}

fn skipped(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules"
                | "target"
                | "dist"
                | "build"
                | "vendor"
                | "Pods"
                | "DerivedData"
                | "__pycache__"
        )
}
fn bare(path: &Path) -> bool {
    path.join("HEAD").is_file() && path.join("objects").is_dir() && path.join("refs").is_dir()
}
fn readable_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
}

pub fn discover(selected: &[PathBuf]) -> Discovery {
    let mut result = Discovery::default();
    let mut seen = HashSet::new();
    let mut added = HashSet::new();
    let mut entries = 0;
    let start = Instant::now();
    for selected in selected {
        let root = match fs::canonicalize(selected) {
            Ok(p) if p.is_dir() => p,
            _ => {
                result
                    .warnings
                    .push(format!("Unavailable folder: {}", selected.display()));
                continue;
            }
        };
        let before = result.projects.len();
        let mut found = added.iter().any(|path: &PathBuf| path.starts_with(&root));
        let mut complete = true;
        let mut root_limited = false;
        let mut stack = vec![(root.clone(), 0)];
        while let Some((path, depth)) = stack.pop() {
            if entries >= MAX_ENTRIES
                || result.projects.len() >= MAX_REPOSITORIES
                || start.elapsed() >= Duration::from_secs(8)
            {
                result.limited = true;
                root_limited = true;
                break;
            }
            if !seen.insert(path.clone()) {
                if added.contains(&path) {
                    found = true;
                }
                continue;
            }
            if bare(&path) {
                if path == root {
                    result.warnings.push(format!(
                        "Bare repository has no working tree: {}",
                        path.display()
                    ));
                    found = true;
                }
                continue;
            }
            if path.join(".git").exists() && crate::git::is_repository_root(&path) {
                found = true;
                if added.insert(path.clone()) {
                    result.projects.push(describe(path, true));
                }
                continue;
            }
            if depth == MAX_DEPTH {
                result.limited = true;
                root_limited = true;
                continue;
            }
            let children = match fs::read_dir(&path) {
                Ok(children) => children,
                Err(_) => {
                    complete = false;
                    result
                        .warnings
                        .push(format!("Cannot inspect folder: {}", path.display()));
                    continue;
                }
            };
            let mut folders = Vec::new();
            for child in children {
                entries += 1;
                if entries >= MAX_ENTRIES || start.elapsed() >= Duration::from_secs(8) {
                    result.limited = true;
                    root_limited = true;
                    break;
                }
                let Ok(child) = child else {
                    complete = false;
                    continue;
                };
                if child
                    .file_type()
                    .is_ok_and(|t| t.is_dir() && !t.is_symlink())
                    && !skipped(&child.file_name().to_string_lossy())
                {
                    folders.push(child.path());
                }
            }
            folders.sort();
            stack.extend(folders.into_iter().rev().map(|p| (p, depth + 1)));
        }
        // Keep the previous ability to open an ordinary non-Git project. Never
        // turn a partially scanned collection into an over-broad project root.
        if !found
            && complete
            && result.projects.len() == before
            && !root_limited
            && !bare(&root)
            && added.insert(root.clone())
        {
            result.projects.push(describe(root, false));
        }
        if result.limited
            && (entries >= MAX_ENTRIES
                || result.projects.len() >= MAX_REPOSITORIES
                || start.elapsed() >= Duration::from_secs(8))
        {
            break;
        }
    }
    result
}
fn describe(path: PathBuf, repository: bool) -> LocalProject {
    let monorepo = repository.then(|| monorepo(&path)).flatten();
    LocalProject {
        path,
        branch: None,
        monorepo,
    }
}
fn text(root: &Path, name: &str) -> Option<String> {
    let path = root.join(name);
    let meta = fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_MANIFEST {
        return None;
    }
    let mut bytes = Vec::new();
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(path)
        .ok()?
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_MANIFEST {
        return None;
    }
    String::from_utf8(bytes).ok()
}
pub fn monorepo(root: &Path) -> Option<String> {
    if let Some(json) =
        text(root, "package.json").and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    {
        let workspaces = &json["workspaces"];
        if workspaces
            .as_array()
            .or_else(|| workspaces["packages"].as_array())
            .is_some_and(|a| {
                a.iter()
                    .any(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
            })
        {
            return Some("JavaScript workspaces".into());
        }
    }
    if let Some(yaml) = text(root, "pnpm-workspace.yaml") {
        let mut packages = false;
        for line in yaml
            .lines()
            .map(|s| s.split('#').next().unwrap_or("").trim_end())
            .filter(|s| !s.trim().is_empty())
        {
            if let Some(value) = line.strip_prefix("packages:") {
                packages = true;
                if value.trim().starts_with('[')
                    && value.chars().any(|c| c.is_alphanumeric() || c == '*')
                {
                    return Some("pnpm workspace".into());
                }
            } else if packages
                && line.trim_start().starts_with('-')
                && workspace_pattern(line.trim_start().trim_start_matches('-').trim())
            {
                return Some("pnpm workspace".into());
            } else if !line.starts_with(char::is_whitespace) {
                packages = false;
            }
        }
    }
    if text(root, "Cargo.toml")
        .and_then(|s| s.parse::<toml::Value>().ok())
        .and_then(|v| v.get("workspace")?.get("members")?.as_array().cloned())
        .is_some_and(|a| {
            a.iter()
                .any(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
        })
    {
        return Some("Cargo workspace".into());
    }
    if let Some(go) = text(root, "go.work") {
        let mut block = false;
        let mut members = 0;
        for line in go
            .lines()
            .map(|s| s.split("//").next().unwrap_or("").trim())
            .filter(|s| !s.is_empty())
        {
            if line == "use (" {
                block = true;
            } else if line == ")" {
                block = false;
            } else if line.starts_with("use ") || block {
                members += 1;
            }
        }
        if members >= 2 {
            return Some("Go workspace".into());
        }
    }
    if text(root, "nx.json").is_some()
        || text(root, "turbo.json").is_some()
        || text(root, "lerna.json").is_some()
    {
        let mut packages = 0;
        for directory in ["packages", "apps", "libs"] {
            let path = root.join(directory);
            if !readable_directory(&path) {
                continue;
            }
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.take(256).flatten() {
                    if readable_directory(&entry.path())
                        && text(&entry.path(), "package.json").is_some()
                    {
                        packages += 1;
                        if packages >= 2 {
                            return Some("Multi-package workspace".into());
                        }
                    }
                }
            }
        }
    }
    None
}

fn workspace_pattern(pattern: &str) -> bool {
    let pattern = pattern.trim().trim_matches(['\'', '"']);
    !pattern.starts_with('!') && pattern.chars().any(|c| c.is_alphanumeric() || c == '*')
}

#[cfg(test)]
mod tests {
    use super::{discover, monorepo};
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
    };
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("dunlin-discovery-{}", uuid::Uuid::now_v7()));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn repo(&self, name: &str) -> PathBuf {
            let p = self.0.join(name);
            fs::create_dir_all(&p).unwrap();
            git(&p, &["init", "-q", "-b", "main"]);
            p
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn git(path: &Path, args: &[&str]) {
        let result = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[test]
    fn collection_imports_all_repositories_once_and_never_imports_the_collection_itself() {
        let f = Fixture::new();
        let one = f.repo("one");
        let two = f.repo("nested/Zażółć repo");
        fs::create_dir(f.0.join("ordinary")).unwrap();
        let found = discover(&[f.0.clone(), f.0.clone(), one.clone(), two.clone()]);
        assert!(!found.limited);
        assert!(found.warnings.is_empty());
        assert_eq!(found.projects.len(), 2);
        let paths: std::collections::HashSet<_> =
            found.projects.iter().map(|p| p.path.clone()).collect();
        assert!(paths.contains(&fs::canonicalize(one).unwrap()));
        assert!(paths.contains(&fs::canonicalize(two).unwrap()));
        assert!(!paths.contains(&f.0));
    }
    #[test]
    fn repository_and_monorepo_roots_stop_traversal_before_nested_repositories() {
        let f = Fixture::new();
        let root = f.repo("mono");
        f.repo("mono/packages/nested");
        fs::write(
            root.join("package.json"),
            r#"{"private":true,"workspaces":["packages/*"]}"#,
        )
        .unwrap();
        let before = fs::read(root.join(".git/HEAD")).unwrap();
        let result = discover(std::slice::from_ref(&root));
        assert_eq!(result.projects.len(), 1);
        assert!(result.projects[0].monorepo.is_some());
        assert_eq!(fs::read(root.join(".git/HEAD")).unwrap(), before);
        assert!(!root.join(".git/FETCH_HEAD").exists());
        assert!(!root.join("AGENTS.md").exists());
    }
    #[test]
    fn worktree_git_files_are_discovered_without_reinterpreting_them_as_collections() {
        let f = Fixture::new();
        let root = f.repo("source");
        fs::write(root.join("one"), "fixture").unwrap();
        git(&root, &["add", "one"]);
        git(
            &root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-qm",
                "fixture",
            ],
        );
        let worktree = f.0.join("worktree");
        git(
            &root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                worktree.to_str().unwrap(),
            ],
        );
        assert!(worktree.join(".git").is_file());
        let found = discover(std::slice::from_ref(&worktree));
        assert_eq!(found.projects.len(), 1);
        assert_eq!(found.projects[0].path, fs::canonicalize(worktree).unwrap());
    }
    #[test]
    fn ordinary_projects_stay_supported_but_partial_scans_never_add_a_parent_workspace() {
        let f = Fixture::new();
        fs::write(f.0.join("readme.md"), "not a repository").unwrap();
        let ordinary = discover(std::slice::from_ref(&f.0));
        assert_eq!(ordinary.projects.len(), 1);
        let mut deep = f.0.clone();
        for _ in 0..super::MAX_DEPTH + 1 {
            deep = deep.join("nested");
            fs::create_dir(&deep).unwrap();
        }
        let partial = discover(std::slice::from_ref(&f.0));
        assert!(partial.limited);
        assert!(partial.projects.is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn implicit_symlinks_and_generated_folders_do_not_widen_the_scan() {
        let f = Fixture::new();
        let external = Fixture::new();
        let outside = external.repo("outside");
        std::os::unix::fs::symlink(&outside, f.0.join("linked")).unwrap();
        f.repo("node_modules/ignored");
        let expected = f.repo("visible");
        let found = discover(std::slice::from_ref(&f.0));
        assert_eq!(found.projects.len(), 1);
        assert_eq!(found.projects[0].path, fs::canonicalize(expected).unwrap());
        let explicit = discover(&[f.0.join("linked"), outside]);
        assert_eq!(explicit.projects.len(), 1);
    }
    #[test]
    fn missing_and_bare_repositories_are_reported_without_importing_fake_workspaces() {
        let f = Fixture::new();
        let bare = f.0.join("bare.git");
        fs::create_dir(&bare).unwrap();
        git(&bare, &["init", "--bare", "-q"]);
        let found = discover(&[bare, f.0.join("missing")]);
        assert!(found.projects.is_empty());
        assert_eq!(found.warnings.len(), 2);
    }
    #[test]
    fn monorepo_badges_require_workspace_evidence_and_do_not_execute_configuration() {
        let f = Fixture::new();
        fs::write(f.0.join("package.json"), r#"{"workspaces":[]}"#).unwrap();
        assert!(monorepo(&f.0).is_none());
        fs::write(f.0.join("package.json"), r#"{"workspaces":{"packages":["apps/*","packages/*"]},"scripts":{"postinstall":"touch MUST_NOT_EXIST"}}"#).unwrap();
        assert_eq!(monorepo(&f.0).as_deref(), Some("JavaScript workspaces"));
        fs::remove_file(f.0.join("package.json")).unwrap();
        fs::write(
            f.0.join("pnpm-workspace.yaml"),
            "packages:\n  - 'packages/*'\n",
        )
        .unwrap();
        assert_eq!(monorepo(&f.0).as_deref(), Some("pnpm workspace"));
        fs::remove_file(f.0.join("pnpm-workspace.yaml")).unwrap();
        fs::write(
            f.0.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();
        assert_eq!(monorepo(&f.0).as_deref(), Some("Cargo workspace"));
        fs::remove_file(f.0.join("Cargo.toml")).unwrap();
        fs::write(f.0.join("go.work"), "go 1.24\nuse (\n./one\n./two\n)\n").unwrap();
        assert_eq!(monorepo(&f.0).as_deref(), Some("Go workspace"));
        assert!(!f.0.join("MUST_NOT_EXIST").exists());
    }
    #[cfg(unix)]
    #[test]
    fn workspace_manifests_cannot_follow_symlinks_or_read_unbounded_content() {
        let f = Fixture::new();
        let external = Fixture::new();
        fs::write(external.0.join("package.json"), r#"{"workspaces":["*"]}"#).unwrap();
        std::os::unix::fs::symlink(external.0.join("package.json"), f.0.join("package.json"))
            .unwrap();
        assert!(monorepo(&f.0).is_none());
        fs::remove_file(f.0.join("package.json")).unwrap();
        fs::write(
            f.0.join("package.json"),
            "x".repeat(super::MAX_MANIFEST as usize + 1),
        )
        .unwrap();
        assert!(monorepo(&f.0).is_none());
    }
}
