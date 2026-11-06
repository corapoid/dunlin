//! Preview-only generated-folder discovery; application supplies recoverable Trash.
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
}
fn identity(path: &Path) -> Result<Identity, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "Cleanup path is unavailable")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Cleanup refuses symlinks and non-directories".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(Identity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        Err("Safe generated-folder cleanup is unavailable on this platform".into())
    }
}
#[derive(Clone, Debug)]
struct Candidate {
    requested: PathBuf,
    root: PathBuf,
    root_identity: Identity,
    relative: PathBuf,
    identity: Identity,
}
#[derive(Clone, Debug, Default)]
pub struct Plan {
    candidates: Vec<Candidate>,
}
fn generated(path: &Path) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    matches!(
        name,
        "node_modules" | ".turbo" | ".pytest_cache" | "__pycache__"
    ) || (name == "cache"
        && path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|s| s == ".next"))
}
fn eligible(root: &Path, relative: &Path) -> Result<bool, String> {
    if !generated(relative)
        || relative.is_absolute()
        || relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Ok(false);
    }
    let mut path = root.to_path_buf();
    for part in relative.components() {
        path.push(part);
        identity(&path)?;
    }
    let root = root.to_str().ok_or("Unsupported cleanup root path")?;
    let relative = relative.to_str().ok_or("Unsupported cleanup folder path")?;
    let tracked = crate::git::activity::git_limited(
        root,
        &[
            "-c",
            "core.fsmonitor=false",
            "--literal-pathspecs",
            "ls-files",
            "-z",
            "--",
            relative,
        ],
        4 * 1024 * 1024,
    )?;
    if !tracked.is_empty() {
        return Ok(false);
    }
    // check-ignore takes literal paths and rejects --literal-pathspecs itself.
    Ok(crate::git::activity::git_limited(
        root,
        &[
            "-c",
            "core.fsmonitor=false",
            "check-ignore",
            "--quiet",
            "--",
            relative,
        ],
        1024,
    )
    .is_ok())
}
pub fn preview(roots: &[PathBuf]) -> Result<Plan, String> {
    let mut plan = Plan::default();
    let mut seen = std::collections::HashSet::new();
    for requested in roots {
        if !requested.is_absolute() {
            return Err("Cleanup requires absolute repository roots".into());
        }
        identity(requested)?;
        let root = fs::canonicalize(requested).map_err(|e| e.to_string())?;
        let canonical = crate::git::history::repository_root(
            root.to_str().ok_or("Unsupported repository path")?,
            None,
        )?;
        if Path::new(&canonical) != root {
            return Err("Cleanup requires the repository root, not a subfolder".into());
        }
        if !seen.insert(root.clone()) {
            continue;
        }
        let root_identity = identity(&root)?;
        let mut stack = vec![(PathBuf::new(), 0)];
        let mut visited = 0;
        while let Some((relative, depth)) = stack.pop() {
            for entry in fs::read_dir(root.join(&relative))
                .map_err(|_| "Could not inspect cleanup folders")?
            {
                visited += 1;
                if visited > 20_000 {
                    return Err(
                        "Cleanup discovery reached its limit. Choose a smaller group.".into(),
                    );
                }
                let entry = entry.map_err(|e| e.to_string())?;
                let kind = entry.file_type().map_err(|e| e.to_string())?;
                if !kind.is_dir() || kind.is_symlink() || entry.file_name() == ".git" {
                    continue;
                }
                let next = relative.join(entry.file_name());
                if entry.path().join(".git").exists() {
                    continue;
                }
                if generated(&next) {
                    if eligible(&root, &next)? {
                        plan.candidates.push(Candidate {
                            requested: requested.clone(),
                            root: root.clone(),
                            root_identity: root_identity.clone(),
                            identity: identity(&entry.path())?,
                            relative: next,
                        });
                    }
                    // Never descend into dependency trees, even when they contain tracked data.
                } else if depth < 16 {
                    stack.push((next, depth + 1));
                }
            }
        }
    }
    Ok(plan)
}
impl Plan {
    pub fn paths(&self) -> Vec<PathBuf> {
        self.candidates
            .iter()
            .map(|c| c.root.join(&c.relative))
            .collect()
    }
    pub fn validate(&self) -> Result<(), String> {
        for c in &self.candidates {
            if fs::canonicalize(&c.requested).ok().as_ref() != Some(&c.root)
                || identity(&c.root)? != c.root_identity
                || identity(&c.root.join(&c.relative))? != c.identity
                || !eligible(&c.root, &c.relative)?
            {
                return Err(
                    "Cleanup target changed. Preview again before moving anything to Trash.".into(),
                );
            }
        }
        Ok(())
    }
    pub fn apply(
        &self,
        mut trash: impl FnMut(&Path) -> Result<(), String>,
    ) -> Result<usize, String> {
        self.validate()?;
        for (done, c) in self.candidates.iter().enumerate() {
            if fs::canonicalize(&c.requested).ok().as_ref() != Some(&c.root)
                || identity(&c.root)? != c.root_identity
                || identity(&c.root.join(&c.relative))? != c.identity
                || !eligible(&c.root, &c.relative)?
            {
                return Err(format!(
                    "Cleanup stopped after {done} folders: a target changed. Earlier folders are in Trash."
                ));
            }
            trash(&c.root.join(&c.relative)).map_err(|error| {
                format!(
                    "Cleanup stopped after {done} folders; earlier folders are in Trash: {error}"
                )
            })?;
        }
        Ok(self.candidates.len())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn git(root: &Path, args: &[&str]) {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    #[test]
    fn cleanup_only_moves_ignored_untracked_generated_folders_and_keeps_source() {
        let repo = tempfile::tempdir().unwrap();
        let root = repo.path();
        git(root, &["init", "--template=", "-q"]);
        fs::write(root.join(".gitignore"), "node_modules/\n.turbo/\n").unwrap();
        for dir in [
            "node_modules",
            "packages/app/node_modules",
            ".turbo",
            "source",
            "dist",
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
            fs::write(root.join(dir).join("keep"), "data").unwrap();
        }
        git(root, &["add", "-f", "packages/app/node_modules/keep"]);
        let plan = preview(&[root.to_path_buf()]).unwrap();
        assert_eq!(plan.paths().len(), 2);
        let trash = tempfile::tempdir().unwrap();
        assert_eq!(
            plan.apply(
                |path| fs::rename(path, trash.path().join(path.file_name().unwrap()))
                    .map_err(|e| e.to_string())
            )
            .unwrap(),
            2
        );
        assert!(root.join("source/keep").exists() && root.join("dist/keep").exists());
        assert!(root.join("packages/app/node_modules/keep").exists());
    }
    #[test]
    fn cleanup_refuses_replaced_symlinked_or_newly_tracked_targets_before_any_move() {
        let repo = tempfile::tempdir().unwrap();
        let root = repo.path();
        git(root, &["init", "--template=", "-q"]);
        fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        fs::create_dir(root.join("node_modules")).unwrap();
        let plan = preview(&[root.to_path_buf()]).unwrap();
        fs::write(root.join("node_modules/source"), "user data").unwrap();
        git(root, &["add", "-f", "node_modules/source"]);
        assert!(
            plan.apply(|_| panic!("must validate every target before Trash"))
                .is_err()
        );
        #[cfg(unix)]
        {
            git(root, &["rm", "--cached", "-q", "node_modules/source"]);
            let plan = preview(&[root.to_path_buf()]).unwrap();
            fs::rename(root.join("node_modules"), root.join("saved")).unwrap();
            std::os::unix::fs::symlink(root.join("saved"), root.join("node_modules")).unwrap();
            assert!(plan.validate().is_err());
            assert!(root.join("saved/source").exists());
        }
    }
    #[test]
    fn cleanup_revalidates_ignores_and_repository_identity_and_leaves_nonignored_folders() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("repo");
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "--template=", "-q"]);
        fs::create_dir(root.join("node_modules")).unwrap();
        assert!(
            preview(std::slice::from_ref(&root))
                .unwrap()
                .paths()
                .is_empty()
        );
        fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        let plan = preview(std::slice::from_ref(&root)).unwrap();
        assert_eq!(plan.paths().len(), 1);
        fs::write(root.join(".gitignore"), "").unwrap();
        assert!(plan.validate().is_err());
        fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        let plan = preview(std::slice::from_ref(&root)).unwrap();
        fs::rename(&root, fixture.path().join("saved-repo")).unwrap();
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "--template=", "-q"]);
        fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        fs::create_dir(root.join("node_modules")).unwrap();
        assert!(
            plan.apply(|_| panic!("stale repository must never be moved"))
                .is_err()
        );
        assert!(fixture.path().join("saved-repo/node_modules").is_dir());
    }
}
