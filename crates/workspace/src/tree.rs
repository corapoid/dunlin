use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub expanded: bool,
    pub depth: u32,
    /// File size, or the recursive size of a directory within this bounded scan.
    #[serde(default)]
    pub size: u64,
    pub children: Vec<FileEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileTree {
    pub root: String,
    pub entries: Vec<FileEntry>,
    pub error: Option<String>,
    /// Sum of the bounded explorer entries, excluding generated directories.
    #[serde(default)]
    pub total_size: u64,
}

impl FileTree {
    pub fn load(root: impl Into<String>) -> Self {
        let root = root.into();
        let path = Path::new(&root);
        if !path.is_dir() {
            return Self {
                root,
                entries: Vec::new(),
                error: Some("This folder is not available on this Mac.".to_string()),
                total_size: 0,
            };
        }

        let mut count = 0;
        let mut incomplete = false;
        let entries = scan_dir(path, 0, &mut count, &mut incomplete);
        let total_size = entries.iter().map(|entry| entry.size).sum();
        Self {
            root,
            entries,
            error: incomplete.then(|| "Explorer scan is incomplete: a size/depth limit or unreadable folder was reached. Open a smaller project folder to see the remaining files.".into()),
            total_size,
        }
    }

    pub fn toggle(&mut self, path: &str) {
        fn visit(entries: &mut [FileEntry], path: &str) {
            for entry in entries {
                if entry.path == path {
                    entry.expanded = !entry.expanded;
                    return;
                }
                visit(&mut entry.children, path);
            }
        }
        visit(&mut self.entries, path);
    }

    pub fn collapse_all(&mut self) {
        fn visit(entries: &mut [FileEntry]) {
            for entry in entries {
                entry.expanded = false;
                visit(&mut entry.children);
            }
        }
        visit(&mut self.entries);
    }

    /// Paths of folders the user has opened, used to preserve state across a
    /// local or remote rescan.
    pub fn expanded_paths(&self) -> Vec<String> {
        fn collect(entries: &[FileEntry], paths: &mut Vec<String>) {
            for entry in entries {
                if entry.expanded {
                    paths.push(entry.path.clone());
                }
                collect(&entry.children, paths);
            }
        }
        let mut paths = Vec::new();
        collect(&self.entries, &mut paths);
        paths
    }

    pub fn restore_expanded(&mut self, paths: &[String]) {
        fn apply(entries: &mut [FileEntry], paths: &[String]) {
            for entry in entries {
                entry.expanded = paths.contains(&entry.path);
                apply(&mut entry.children, paths);
            }
        }
        apply(&mut self.entries, paths);
    }

    /// Rescan the tree while keeping the folders the user opened.
    pub fn reload(&mut self, root: impl Into<String>) {
        use std::collections::HashSet;

        fn collect(entries: &[FileEntry], expanded: &mut HashSet<String>) {
            for entry in entries {
                if entry.expanded {
                    expanded.insert(entry.path.clone());
                }
                collect(&entry.children, expanded);
            }
        }
        fn apply(entries: &mut [FileEntry], expanded: &HashSet<String>) {
            for entry in entries {
                entry.expanded = expanded.contains(&entry.path);
                apply(&mut entry.children, expanded);
            }
        }

        let mut expanded = HashSet::new();
        collect(&self.entries, &mut expanded);
        let mut next = Self::load(root);
        apply(&mut next.entries, &expanded);
        *self = next;
    }
}

const MAX_ENTRIES: usize = 20_000;

/// Whole-folder allocated storage, including generated files and Git metadata.
/// `du -P` counts symlinks themselves, never their targets; incomplete scans stay unknown.
pub fn storage_size(root: &Path) -> Option<u64> {
    let root = fs::canonicalize(root).ok()?;
    if !root.is_dir() {
        return None;
    }
    let mut command = std::process::Command::new("/usr/bin/du");
    command
        .args(["-s", "-k", "-P"])
        .arg(&root)
        .env("LC_ALL", "C");
    let (status, bytes) =
        crate::github::process::run(command, None, std::time::Duration::from_secs(8), 65536)
            .ok()?;
    if !status.success() {
        return None;
    }
    let value = std::str::from_utf8(&bytes)
        .ok()?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    value.checked_mul(1024)
}

fn is_skipped(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".DS_Store"
            | "target"
            | "node_modules"
            | ".next"
            | "dist"
            | ".turbo"
            | ".idea"
            | ".vscode"
    ) || (name.starts_with('.') && name != ".gitignore" && name != ".github")
}

fn scan_dir(dir: &Path, depth: u32, count: &mut usize, incomplete: &mut bool) -> Vec<FileEntry> {
    if depth > 64 || *count >= MAX_ENTRIES {
        *incomplete = true;
        return Vec::new();
    }

    let Ok(read) = fs::read_dir(dir) else {
        *incomplete = true;
        return Vec::new();
    };

    let mut paths: Vec<PathBuf> = read
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry.path()),
            Err(_) => {
                *incomplete = true;
                None
            }
        })
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_none_or(|name| !is_skipped(name))
        })
        .take(MAX_ENTRIES + 1)
        .collect();
    if paths.len() > MAX_ENTRIES {
        *incomplete = true;
        paths.truncate(MAX_ENTRIES);
    }
    paths.sort_by(|a, b| {
        let a_dir = a.is_dir();
        let b_dir = b.is_dir();
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.file_name().cmp(&b.file_name()))
    });

    let mut entries = Vec::new();
    for path in paths {
        if *count >= MAX_ENTRIES {
            *incomplete = true;
            break;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_skipped(name) {
            continue;
        }
        // A symlink may point outside the project or back to an ancestor.
        // Show it as a leaf instead of traversing it during Explorer scans.
        let is_dir = path.is_dir() && !path.is_symlink();
        let expanded = depth == 0 && matches!(name, "apps" | "crates" | "assets");
        *count += 1;
        let children = if is_dir {
            scan_dir(&path, depth + 1, count, incomplete)
        } else {
            Vec::new()
        };
        let size = if is_dir {
            children.iter().map(|child| child.size).sum()
        } else {
            fs::symlink_metadata(&path)
                .map(|metadata| metadata.len())
                .unwrap_or(0)
        };
        entries.push(FileEntry {
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            is_dir,
            expanded,
            depth,
            size,
            children,
        });
    }
    entries
}

/// Bounded source-size scan used by project tiles. It shares the Explorer's
/// generated-directory exclusions, does not follow symlinks and caps traversal.
pub fn project_size(root: &Path) -> Option<u64> {
    if !root.is_dir() {
        return None;
    }
    let mut total = 0u64;
    let mut count = 0usize;
    size_dir(root, 0, &mut total, &mut count);
    Some(total)
}

fn size_dir(dir: &Path, depth: u32, total: &mut u64, count: &mut usize) {
    if depth > 12 || *count >= 100_000 {
        return;
    }
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        if *count >= 100_000 {
            break;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if is_skipped(name) {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            size_dir(&path, depth + 1, total, count);
        } else if metadata.is_file() {
            *count += 1;
            *total = total.saturating_add(metadata.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FileTree;
    use std::fs;

    #[test]
    fn wide_and_deep_directories_are_not_silently_hidden() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..120 {
            fs::write(root.path().join(format!("file-{index:03}.rs")), "").unwrap();
        }
        let deep = root.path().join("a/b/c/d/e/f");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("visible.rs"), "").unwrap();
        let tree = FileTree::load(root.path().to_string_lossy());
        assert!(tree.error.is_none());
        assert_eq!(
            tree.entries.iter().filter(|entry| !entry.is_dir).count(),
            120
        );
        let mut entry = tree.entries.iter().find(|entry| entry.name == "a").unwrap();
        for name in ["b", "c", "d", "e", "f", "visible.rs"] {
            entry = entry
                .children
                .iter()
                .find(|entry| entry.name == name)
                .unwrap();
        }
    }

    #[test]
    fn scan_limits_are_reported_explicitly() {
        let root = tempfile::tempdir().unwrap();
        let mut count = super::MAX_ENTRIES;
        let mut incomplete = false;
        assert!(super::scan_dir(root.path(), 0, &mut count, &mut incomplete).is_empty());
        assert!(incomplete);
    }

    #[cfg(unix)]
    #[test]
    fn storage_includes_generated_and_hidden_files_without_following_links() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("node_modules")).unwrap();
        fs::create_dir(root.path().join(".git")).unwrap();
        fs::write(root.path().join("source.rs"), "fn main() {}").unwrap();
        fs::write(root.path().join("node_modules/generated"), vec![1; 131_072]).unwrap();
        fs::write(root.path().join(".git/object"), vec![1; 65_536]).unwrap();
        fs::write(outside.path().join("external"), vec![1; 2_097_152]).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("outside-link")).unwrap();
        let storage = super::storage_size(root.path()).unwrap();
        assert!((196_608..2_097_152).contains(&storage), "storage={storage}");
        assert!(super::project_size(root.path()).unwrap() < storage);
        assert!(super::storage_size(&root.path().join("missing")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn directory_symlinks_are_leaves_and_never_scanned() {
        let root = std::env::temp_dir().join(format!("dunlin-tree-links-{}", uuid::Uuid::now_v7()));
        fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();
        let tree = FileTree::load(root.to_string_lossy().to_string());
        let link = tree
            .entries
            .iter()
            .find(|entry| entry.name == "loop")
            .unwrap();
        assert!(!link.is_dir);
        assert!(link.children.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reload_keeps_open_folders_and_sees_new_files() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dunlin-tree-{unique}"));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "").unwrap();

        let path = root.to_string_lossy().to_string();
        let mut tree = FileTree::load(path.clone());
        for entry in &mut tree.entries {
            if entry.name == "src" {
                entry.expanded = true;
            }
        }

        fs::write(root.join("new.txt"), "").unwrap();
        tree.reload(path);

        assert!(tree.entries.iter().any(|entry| entry.name == "new.txt"));
        assert!(
            tree.entries
                .iter()
                .find(|entry| entry.name == "src")
                .unwrap()
                .expanded
        );

        let _ = fs::remove_dir_all(&root);
    }
    #[test]
    fn project_size_skips_generated_directories_and_file_entry_sizes_roll_up() {
        let root = std::env::temp_dir().join(format!("dunlin-tree-size-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::write(root.join("src/main.rs"), vec![b'x'; 120]).unwrap();
        fs::write(root.join("README.md"), vec![b'y'; 30]).unwrap();
        fs::write(root.join("target/generated.rs"), vec![b'z'; 10_000]).unwrap();

        let tree = FileTree::load(root.to_string_lossy().to_string());
        assert_eq!(tree.total_size, 150);
        let src = tree
            .entries
            .iter()
            .find(|entry| entry.name == "src")
            .unwrap();
        assert_eq!(src.size, 120);
        assert_eq!(super::project_size(&root), Some(150));
        fs::remove_dir_all(root).unwrap();
    }
}
