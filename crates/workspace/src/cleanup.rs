//! Bounded generated-folder inspection. The application supplies system Trash;
//! this module never deletes files or executes project-provided clean commands.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dependencies,
    Cache,
    Build,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dependencies => "Dependencies",
            Self::Cache => "Cache",
            Self::Build => "Build output",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Folder {
    pub path: PathBuf,
    pub kind: Kind,
    /// Logical file sizes, not a promise of immediately reclaimable disk space.
    pub bytes: u64,
}

#[derive(Clone, Debug)]
pub struct Skipped {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Default)]
pub struct Preview {
    pub plan: Plan,
    pub skipped: Vec<Skipped>,
}

#[derive(Default)]
pub struct Outcome {
    pub moved: Vec<PathBuf>,
    pub skipped: Vec<Skipped>,
}

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
    kind: Kind,
    bytes: u64,
}
#[derive(Clone, Debug, Default)]
pub struct Plan {
    candidates: Vec<Candidate>,
}
fn marker(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
}

fn generated(root: &Path, path: &Path) -> Option<Kind> {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    match name {
        "node_modules" => Some(Kind::Dependencies),
        ".turbo" | ".pytest_cache" | "__pycache__" => Some(Kind::Cache),
        "cache"
            if path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|s| s == ".next") =>
        {
            Some(Kind::Cache)
        }
        // Generic names alone are never sufficient evidence of build output.
        "target" if marker(&root.join(path.parent()?).join("Cargo.toml")) => Some(Kind::Build),
        "dist" | "build" | ".next" | ".nuxt" | ".output" | ".svelte-kit"
            if marker(&root.join(path.parent()?).join("package.json")) =>
        {
            Some(Kind::Build)
        }
        _ => None,
    }
}
fn eligible(root: &Path, relative: &Path) -> Result<bool, String> {
    if generated(root, relative).is_none()
        || relative.is_absolute()
        || relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Ok(false);
    }
    let mut path = root.to_path_buf();
    let device = identity(root)?.device;
    for part in relative.components() {
        path.push(part);
        if identity(&path)?.device != device || path.join(".git").symlink_metadata().is_ok() {
            return Err("Cleanup refuses mounted folders and nested repositories".into());
        }
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

/// Inspect contents without following links, crossing devices, or including Git
/// metadata. Limits are fail-closed: an incomplete inspection is not selectable.
fn contents(path: &Path, cancel: &AtomicBool) -> Result<u64, String> {
    let device = identity(path)?.device;
    let started = Instant::now();
    let mut stack = vec![path.to_path_buf()];
    let mut visited = 0;
    let mut bytes = 0u64;
    while let Some(directory) = stack.pop() {
        if identity(&directory)?.device != device {
            return Err("Folder contains a mount point".into());
        }
        for entry in fs::read_dir(&directory).map_err(|_| "Could not inspect folder contents")? {
            visited += 1;
            if cancel.load(Ordering::Relaxed) {
                return Err("Inspection cancelled".into());
            }
            if visited > 1_000_000 || started.elapsed() > Duration::from_secs(15) {
                return Err("Folder inspection reached its safety limit".into());
            }
            let entry = entry.map_err(|_| "Could not inspect a folder entry")?;
            if entry.file_name() == ".git" {
                return Err("Folder contains Git metadata or a nested repository".into());
            }
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|_| "Folder contents changed or are inaccessible")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if metadata.dev() != device {
                    return Err("Folder contains a mount point".into());
                }
            }
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                stack.push(entry.path());
            } else {
                bytes = bytes.saturating_add(metadata.len());
            }
        }
    }
    Ok(bytes)
}

pub fn preview(roots: &[PathBuf]) -> Result<Plan, String> {
    let mut plan = Plan::default();
    let mut seen = std::collections::HashSet::new();
    let cancel = AtomicBool::new(false);
    for requested in roots {
        let mut skipped = Vec::new();
        let next = inspect_root(requested, &cancel, &mut skipped)?;
        for candidate in next.candidates {
            if seen.insert(candidate.root.join(&candidate.relative)) {
                plan.candidates.push(candidate);
            }
        }
    }
    plan.sort();
    Ok(plan)
}

/// A missing/inaccessible project does not prevent inspecting the rest of a group.
pub fn inspect(roots: &[PathBuf], cancel: &AtomicBool) -> Preview {
    let mut result = Preview::default();
    let mut seen = std::collections::HashSet::new();
    for requested in roots {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let key = fs::canonicalize(requested).unwrap_or_else(|_| requested.clone());
        if seen.contains(&key) {
            continue;
        }
        match inspect_root(requested, cancel, &mut result.skipped) {
            Ok(plan) => {
                seen.insert(key);
                result.plan.candidates.extend(plan.candidates);
            }
            Err(reason) => result.skipped.push(Skipped {
                path: requested.clone(),
                reason,
            }),
        }
    }
    result.plan.sort();
    result
}

fn inspect_root(
    requested: &Path,
    cancel: &AtomicBool,
    skipped: &mut Vec<Skipped>,
) -> Result<Plan, String> {
    let mut plan = Plan::default();
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
    let root_identity = identity(&root)?;
    let mut stack = vec![(PathBuf::new(), 0)];
    let mut visited = 0;
    let started = Instant::now();
    while let Some((relative, depth)) = stack.pop() {
        // Recheck traversed ancestors too, not just the eventual candidates.
        let mut ancestor = root.clone();
        for part in relative.components() {
            ancestor.push(part);
            if identity(&ancestor)?.device != root_identity.device {
                return Err("Cleanup refuses mounted folders".into());
            }
        }
        for entry in
            fs::read_dir(root.join(&relative)).map_err(|_| "Could not inspect cleanup folders")?
        {
            visited += 1;
            if cancel.load(Ordering::Relaxed) {
                return Err("Inspection cancelled".into());
            }
            if visited > 20_000 || started.elapsed() > Duration::from_secs(60) {
                return Err("Cleanup discovery reached its limit. Choose a smaller group.".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                if generated(&root, &relative.join(entry.file_name())).is_some() {
                    skipped.push(Skipped {
                        path: entry.path(),
                        reason: "Symbolic link; not followed".into(),
                    });
                }
                continue;
            }
            if !kind.is_dir() || entry.file_name() == ".git" {
                continue;
            }
            let next = relative.join(entry.file_name());
            if entry.path().join(".git").symlink_metadata().is_ok() {
                skipped.push(Skipped {
                    path: entry.path(),
                    reason: "Nested repository".into(),
                });
                continue;
            }
            if let Some(kind) = generated(&root, &next) {
                let inspected = (|| {
                    if !eligible(&root, &next)? {
                        return Err("Not ignored by Git, or contains tracked files".into());
                    }
                    let before = identity(&entry.path())?;
                    let bytes = contents(&entry.path(), cancel)?;
                    if identity(&entry.path())? != before {
                        return Err("Folder changed during inspection".into());
                    }
                    Ok(Candidate {
                        requested: requested.to_path_buf(),
                        root: root.clone(),
                        root_identity: root_identity.clone(),
                        identity: before,
                        relative: next,
                        kind,
                        bytes,
                    })
                })();
                match inspected {
                    Ok(candidate) => plan.candidates.push(candidate),
                    Err(reason) => skipped.push(Skipped {
                        path: entry.path(),
                        reason,
                    }),
                }
                // Never descend into dependency trees, even when they contain tracked data.
            } else if depth < 16 {
                stack.push((next, depth + 1));
            } else {
                skipped.push(Skipped {
                    path: entry.path(),
                    reason: "Discovery depth limit reached".into(),
                });
            }
        }
    }
    Ok(plan)
}

impl Candidate {
    fn validate_identity(&self) -> Result<(), String> {
        if fs::canonicalize(&self.requested).ok().as_ref() != Some(&self.root)
            || identity(&self.root)? != self.root_identity
            || identity(&self.root.join(&self.relative))? != self.identity
            || !eligible(&self.root, &self.relative)?
        {
            return Err("Cleanup target changed. Inspect again before moving it to Trash.".into());
        }
        let canonical = crate::git::history::repository_root(
            self.root.to_str().ok_or("Unsupported repository path")?,
            None,
        )?;
        if Path::new(&canonical) != self.root {
            return Err("Repository root changed".into());
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), String> {
        self.validate_identity()?;
        contents(&self.root.join(&self.relative), &AtomicBool::new(false))?;
        self.validate_identity()
    }
}

impl Plan {
    fn sort(&mut self) {
        self.candidates.sort_by_key(|c| c.root.join(&c.relative));
        // Overlapping/duplicate project entries must never move the same data twice.
        let mut paths = std::collections::HashSet::new();
        self.candidates.retain(|c| {
            let path = c.root.join(&c.relative);
            if path.ancestors().any(|ancestor| paths.contains(ancestor)) {
                false
            } else {
                paths.insert(path);
                true
            }
        });
    }

    pub fn folders(&self) -> Vec<Folder> {
        self.candidates
            .iter()
            .map(|c| Folder {
                path: c.root.join(&c.relative),
                kind: c.kind,
                bytes: c.bytes,
            })
            .collect()
    }

    pub fn selected(&self, selected: &[bool]) -> Result<Self, String> {
        if selected.len() != self.candidates.len() {
            return Err("Cleanup selection changed. Inspect again.".into());
        }
        Ok(Self {
            candidates: self
                .candidates
                .iter()
                .zip(selected)
                .filter(|(_, selected)| **selected)
                .map(|(c, _)| c.clone())
                .collect(),
        })
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.candidates
            .iter()
            .map(|c| c.root.join(&c.relative))
            .collect()
    }
    pub fn validate(&self) -> Result<(), String> {
        for c in &self.candidates {
            c.validate()?;
        }
        Ok(())
    }
    pub fn apply(
        &self,
        mut trash: impl FnMut(&Path) -> Result<(), String>,
    ) -> Result<usize, String> {
        self.validate()?;
        for (done, c) in self.candidates.iter().enumerate() {
            if c.validate().is_err() {
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

    /// Best-effort batch with per-folder outcomes, preserving successful Trash
    /// moves even if a later project disappears or the system Trash reports an error.
    pub fn apply_available(&self, mut trash: impl FnMut(&Path) -> Result<(), String>) -> Outcome {
        let mut outcome = Outcome::default();
        for c in &self.candidates {
            let path = c.root.join(&c.relative);
            match c.validate().and_then(|()| trash(&path)) {
                Ok(()) => outcome.moved.push(path),
                Err(reason) => outcome.skipped.push(Skipped { path, reason }),
            }
        }
        outcome
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn repo() -> tempfile::TempDir {
        let repo = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        git(repo.path(), &["init", "--template=", "-q"]);
        repo
    }

    fn folder(root: &Path, path: &str, data: &str) {
        fs::create_dir_all(root.join(path)).unwrap();
        fs::write(root.join(path).join("fixture"), data).unwrap();
    }

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

    #[test]
    fn builds_need_a_matching_manifest_and_git_ignores_and_sizes_are_measured() {
        let repo = repo();
        let root = repo.path();
        fs::write(
            root.join(".gitignore"),
            "target/\ndist/\nbuild/\n.next/\nnode_modules/\n",
        )
        .unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
        fs::write(root.join("package.json"), "{}").unwrap();
        for path in [
            "target",
            "dist",
            "build",
            ".next",
            "node_modules",
            "unknown/target",
            "unknown/dist",
            "source",
        ] {
            folder(root, path, "12345");
        }
        fs::write(root.join(".next/extra"), "123").unwrap();
        // Monorepo packages are recognized independently of the root manifest.
        folder(root, "packages/app/dist", "12");
        fs::write(root.join("packages/app/package.json"), "{}").unwrap();
        let plan = preview(&[root.to_path_buf()]).unwrap();
        let folders = plan.folders();
        assert_eq!(folders.len(), 6);
        assert_eq!(folders.iter().map(|f| f.bytes).sum::<u64>(), 30);
        assert_eq!(folders.iter().filter(|f| f.kind == Kind::Build).count(), 5);
        assert!(!plan.paths().contains(&root.join("unknown/dist")));
        assert!(!plan.paths().contains(&root.join("unknown/target")));
        // Manifest removal invalidates an already reviewed build target.
        fs::remove_file(root.join("Cargo.toml")).unwrap();
        assert!(plan.validate().is_err());
    }

    #[test]
    fn mixed_roots_are_deduplicated_and_missing_or_non_repository_projects_are_reported() {
        let repo = repo();
        let root = repo.path();
        let non_repo = tempfile::tempdir().unwrap();
        fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        folder(root, "node_modules", "data");
        let preview = inspect(
            &[
                root.to_path_buf(),
                root.to_path_buf(),
                root.join("missing"),
                non_repo.path().to_path_buf(),
            ],
            &AtomicBool::new(false),
        );
        assert_eq!(preview.plan.paths(), vec![root.join("node_modules")]);
        assert_eq!(preview.skipped.len(), 2);
        assert!(
            preview
                .skipped
                .iter()
                .any(|s| s.path == root.join("missing"))
        );
        // Cancellation performs no inspection and never offers partial data.
        let cancelled = inspect(&[root.to_path_buf()], &AtomicBool::new(true));
        assert!(cancelled.plan.paths().is_empty());
    }

    #[test]
    fn selection_only_moves_checked_folders_and_rejects_stale_selection_lengths() {
        let repo = repo();
        let root = repo.path();
        fs::write(root.join(".gitignore"), "node_modules/\n.turbo/\n").unwrap();
        folder(root, "node_modules", "dependency");
        folder(root, ".turbo", "cache");
        let plan = preview(&[root.to_path_buf()]).unwrap();
        assert!(plan.selected(&[true]).is_err());
        let selected = plan.selected(&[false, true]).unwrap();
        assert_eq!(selected.paths(), vec![root.join("node_modules")]);
        let trash = tempfile::tempdir().unwrap();
        let outcome = selected.apply_available(|path| {
            fs::rename(path, trash.path().join("dependencies")).map_err(|e| e.to_string())
        });
        assert_eq!(outcome.moved, vec![root.join("node_modules")]);
        assert!(outcome.skipped.is_empty());
        assert!(root.join(".turbo/fixture").exists());
        assert_eq!(
            fs::read_to_string(trash.path().join("dependencies/fixture")).unwrap(),
            "dependency"
        );
        let empty = plan.selected(&[false, false]).unwrap();
        assert!(
            empty
                .apply_available(|_| panic!("empty selection must not move anything"))
                .moved
                .is_empty()
        );
    }

    #[test]
    fn batch_continues_after_trash_failure_and_reports_each_actual_outcome() {
        let repo = repo();
        let root = repo.path();
        fs::write(root.join(".gitignore"), "node_modules/\n.turbo/\n").unwrap();
        folder(root, "node_modules", "dependency");
        folder(root, ".turbo", "cache");
        let plan = preview(&[root.to_path_buf()]).unwrap();
        let trash = tempfile::tempdir().unwrap();
        let outcome = plan.apply_available(|path| {
            if path.ends_with(".turbo") {
                return Err("System Trash refused this folder".into());
            }
            fs::rename(path, trash.path().join("dependencies")).map_err(|e| e.to_string())
        });
        assert_eq!(outcome.moved, vec![root.join("node_modules")]);
        assert_eq!(outcome.skipped.len(), 1);
        assert!(outcome.skipped[0].reason.contains("System Trash refused"));
        assert!(root.join(".turbo/fixture").exists());
    }

    #[test]
    fn batch_revalidates_later_folders_after_an_earlier_move() {
        let repo = repo();
        let root = repo.path();
        fs::write(root.join(".gitignore"), "node_modules/\n.turbo/\n").unwrap();
        folder(root, "node_modules", "dependency");
        folder(root, ".turbo", "cache");
        let plan = preview(&[root.to_path_buf()]).unwrap();
        let trash = tempfile::tempdir().unwrap();
        let outcome = plan.apply_available(|path| {
            assert!(path.ends_with(".turbo"));
            fs::rename(path, trash.path().join("cache")).unwrap();
            // A build/install tool turns a later target into a nested repository.
            fs::create_dir(root.join("node_modules/dependency")).unwrap();
            fs::write(
                root.join("node_modules/dependency/.git"),
                "gitdir: elsewhere",
            )
            .unwrap();
            Ok(())
        });
        assert_eq!(outcome.moved, vec![root.join(".turbo")]);
        assert_eq!(outcome.skipped.len(), 1);
        assert!(outcome.skipped[0].reason.contains("Git metadata"));
        assert!(root.join("node_modules/fixture").exists());
    }

    #[test]
    fn tracked_builds_and_nested_git_metadata_are_never_offered() {
        let repo = repo();
        let root = repo.path();
        fs::write(root.join("package.json"), "{}").unwrap();
        fs::write(root.join(".gitignore"), "dist/\nnode_modules/\n").unwrap();
        folder(root, "dist", "tracked source");
        git(root, &["add", "-f", "dist/fixture"]);
        folder(root, "node_modules/dependency/.git", "metadata");
        let result = inspect(&[root.to_path_buf()], &AtomicBool::new(false));
        assert!(result.plan.paths().is_empty());
        assert_eq!(result.skipped.len(), 2);
        assert!(
            result
                .skipped
                .iter()
                .any(|s| s.reason.contains("tracked files"))
        );
        assert!(
            result
                .skipped
                .iter()
                .any(|s| s.reason.contains("Git metadata"))
        );
        assert!(root.join("dist/fixture").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_contents_are_not_followed_and_symlinked_targets_are_not_offered() {
        use std::os::unix::fs::symlink;
        let repo = repo();
        let root = repo.path();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("source"), "outside source must survive").unwrap();
        fs::write(root.join(".gitignore"), "node_modules/\n.turbo/\n").unwrap();
        folder(root, "node_modules", "data");
        symlink(outside.path(), root.join("node_modules/link")).unwrap();
        symlink(outside.path(), root.join(".turbo")).unwrap();
        let plan = preview(&[root.to_path_buf()]).unwrap();
        assert_eq!(plan.paths(), vec![root.join("node_modules")]);
        assert_eq!(
            plan.folders()[0].bytes,
            4 + fs::symlink_metadata(root.join("node_modules/link"))
                .unwrap()
                .len()
        );
        let trash = tempfile::tempdir().unwrap();
        let result = plan.apply_available(|path| {
            fs::rename(path, trash.path().join("dependencies")).map_err(|e| e.to_string())
        });
        assert_eq!(result.moved.len(), 1);
        assert_eq!(
            fs::read_to_string(outside.path().join("source")).unwrap(),
            "outside source must survive"
        );
        assert!(
            root.join(".turbo")
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejected_symlink_root_does_not_hide_a_valid_project_in_the_same_group() {
        let repo = repo();
        let root = repo.path();
        let fixture = tempfile::tempdir().unwrap();
        let alias = fixture.path().join("alias");
        std::os::unix::fs::symlink(root, &alias).unwrap();
        fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        folder(root, "node_modules", "data");
        let result = inspect(&[alias, root.to_path_buf()], &AtomicBool::new(false));
        assert_eq!(result.plan.paths(), vec![root.join("node_modules")]);
        assert_eq!(result.skipped.len(), 1);
        assert!(result.skipped[0].reason.contains("symlinks"));
    }

    #[test]
    fn available_cleanup_never_moves_newly_tracked_or_replaced_folders() {
        let repo = repo();
        let root = repo.path();
        fs::write(root.join(".gitignore"), "node_modules/\n.turbo/\n").unwrap();
        folder(root, "node_modules", "source");
        folder(root, ".turbo", "cache");
        let plan = preview(&[root.to_path_buf()]).unwrap();
        git(root, &["add", "-f", "node_modules/fixture"]);
        fs::rename(root.join(".turbo"), root.join("saved-cache")).unwrap();
        folder(root, ".turbo", "replacement");
        let result = plan.apply_available(|_| panic!("changed targets must never reach Trash"));
        assert!(result.moved.is_empty());
        assert_eq!(result.skipped.len(), 2);
        assert_eq!(
            fs::read_to_string(root.join("node_modules/fixture")).unwrap(),
            "source"
        );
        assert_eq!(
            fs::read_to_string(root.join(".turbo/fixture")).unwrap(),
            "replacement"
        );
        assert_eq!(
            fs::read_to_string(root.join("saved-cache/fixture")).unwrap(),
            "cache"
        );
    }

    #[cfg(unix)]
    #[test]
    fn replaced_parent_symlink_is_refused_even_when_target_inode_stays_the_same() {
        let repo = repo();
        let root = repo.path();
        fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        folder(root, "packages/app/node_modules", "keep");
        let plan = preview(&[root.to_path_buf()]).unwrap();
        fs::rename(root.join("packages"), root.join("saved-packages")).unwrap();
        std::os::unix::fs::symlink(root.join("saved-packages"), root.join("packages")).unwrap();
        let result = plan.apply_available(|_| panic!("symlinked ancestors must never reach Trash"));
        assert!(result.moved.is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(
            fs::read_to_string(root.join("saved-packages/app/node_modules/fixture")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn discovery_depth_limit_is_reported_instead_of_offering_uninspected_data() {
        let repo = repo();
        let root = repo.path();
        fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        let deep = format!("{}node_modules", "nested/".repeat(18));
        folder(root, &deep, "keep");
        let result = inspect(&[root.to_path_buf()], &AtomicBool::new(false));
        assert!(result.plan.paths().is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert!(result.skipped[0].reason.contains("depth limit"));
        assert!(root.join(deep).join("fixture").exists());
    }
}
