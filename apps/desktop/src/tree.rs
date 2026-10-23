use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub expanded: bool,
    pub depth: u32,
    pub children: Vec<FileEntry>,
}

#[derive(Clone, Debug)]
pub struct FileTree {
    #[allow(dead_code)]
    pub root: String,
    pub entries: Vec<FileEntry>,
    pub error: Option<String>,
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
            };
        }

        let entries = scan_dir(path, 0);
        Self {
            root,
            entries,
            error: None,
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

fn scan_dir(dir: &Path, depth: u32) -> Vec<FileEntry> {
    if depth > 4 {
        return Vec::new();
    }

    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut paths: Vec<PathBuf> = read
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    paths.sort_by(|a, b| {
        let a_dir = a.is_dir();
        let b_dir = b.is_dir();
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.file_name().cmp(&b.file_name()))
    });

    let mut entries = Vec::new();
    for path in paths.into_iter().take(80) {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_skipped(name) {
            continue;
        }
        let is_dir = path.is_dir();
        let expanded = depth == 0 && matches!(name, "apps" | "crates" | "assets");
        let children = if is_dir {
            scan_dir(&path, depth + 1)
        } else {
            Vec::new()
        };
        entries.push(FileEntry {
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            is_dir,
            expanded,
            depth,
            children,
        });
    }
    entries
}
