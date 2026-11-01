//! Lightweight project-wide text search.
//!
//! The UI hands the walk to GPUI's background executor, so large repositories
//! never block a frame. A search walks the project once, skips build/VCS
//! directories, only reads text files below 256 KiB and stops after `limit`
//! matches. The same implementation runs inside the relay, so remote search
//! behaves exactly like local search.

use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

const MAX_DEPTH: usize = 12;
const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_FILES: usize = 6_000;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub path: String,
    pub line: u32,
    pub preview: String,
}

pub fn search_project(root: &str, query: &str, limit: usize) -> Vec<SearchHit> {
    let query = query.trim();
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }
    let needle = query.to_lowercase();
    let display_root = Path::new(root).to_path_buf();
    let Ok(root) = fs::canonicalize(root) else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    let mut scanned = 0usize;
    let mut bytes_scanned = 0u64;
    walk(
        &root,
        &needle,
        limit,
        &mut scanned,
        &mut bytes_scanned,
        0,
        &mut hits,
    );
    // Keep the caller's spelling of the project root. On macOS `/var` is a
    // symlink to `/private/var`; callers still need paths relative to the path
    // they supplied so `entry_relative` can map results back to the project.
    for hit in &mut hits {
        if let Ok(relative) = Path::new(&hit.path).strip_prefix(&root) {
            hit.path = display_root.join(relative).to_string_lossy().into_owned();
        }
    }
    hits
}

fn walk(
    dir: &Path,
    needle: &str,
    limit: usize,
    scanned: &mut usize,
    bytes_scanned: &mut u64,
    depth: usize,
    hits: &mut Vec<SearchHit>,
) {
    if depth > MAX_DEPTH
        || hits.len() >= limit
        || *scanned >= MAX_FILES
        || *bytes_scanned >= MAX_TOTAL_BYTES
    {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    paths.sort();

    for path in paths {
        if hits.len() >= limit || *scanned >= MAX_FILES || *bytes_scanned >= MAX_TOTAL_BYTES {
            return;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            // A project symlink must never widen search beyond the canonical
            // project root, even when its target exists and is readable.
            continue;
        }
        if metadata.is_dir() {
            if !is_skipped_dir(name) {
                walk(
                    &path,
                    needle,
                    limit,
                    scanned,
                    bytes_scanned,
                    depth + 1,
                    hits,
                );
            }
            continue;
        }
        if is_skipped_file(name) || !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            continue;
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let Ok(file) = options.open(&path) else {
            continue;
        };
        let Ok(meta) = file.metadata() else {
            continue;
        };
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let mut bytes = Vec::new();
        if file
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() as u64 > MAX_FILE_BYTES
        {
            continue;
        }
        *bytes_scanned = bytes_scanned.saturating_add(bytes.len() as u64);
        if *bytes_scanned > MAX_TOTAL_BYTES {
            return;
        }
        if bytes.iter().take(2048).any(|byte| *byte == 0) {
            continue;
        }
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        *scanned += 1;

        for (index, line) in text.lines().enumerate() {
            if !line.to_lowercase().contains(needle) {
                continue;
            }
            let preview = line.trim();
            let preview = if preview.chars().count() > 180 {
                let mut short: String = preview.chars().take(180).collect();
                short.push('…');
                short
            } else {
                preview.to_string()
            };
            hits.push(SearchHit {
                path: path.to_string_lossy().to_string(),
                line: index as u32 + 1,
                preview,
            });
            if hits.len() >= limit {
                return;
            }
        }
    }
}

fn is_skipped_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".hg"
            | ".svn"
            | "target"
            | "node_modules"
            | "dist"
            | "build"
            | ".next"
            | ".turbo"
            | ".cache"
            | "Pods"
            | "DerivedData"
    ) || (name.starts_with('.') && name != ".github" && name != ".config")
}

fn is_skipped_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        ".ds_store"
            | ".env"
            | ".netrc"
            | ".npmrc"
            | ".pypirc"
            | "id_rsa"
            | "id_ed25519"
            | "cargo.lock"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
    ) || lower.starts_with(".env.")
        || lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".webp")
        || lower.ends_with(".pdf")
        || lower.ends_with(".ttf")
        || lower.ends_with(".otf")
        || lower.ends_with(".woff")
        || lower.ends_with(".woff2")
        || lower.ends_with(".zip")
}

#[cfg(test)]
mod tests {
    use super::search_project;
    use std::fs;

    #[test]
    fn search_skips_secret_files() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dunlin-search-{unique}"));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(".env"), "SECRET=1\n").unwrap();
        fs::write(root.join("main.rs"), "let secret = 1;\n").unwrap();

        let hits = search_project(&root.to_string_lossy(), "secret", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].path,
            root.join("main.rs").to_string_lossy().to_string()
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn search_does_not_follow_symlinks_outside_the_project() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("dunlin-search-links-{unique}"));
        let project = base.join("project");
        let outside = base.join("outside");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "AUDIT_SECRET_MARKER\n").unwrap();
        fs::write(project.join("visible.txt"), "AUDIT_SECRET_MARKER\n").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), project.join("leak.txt")).unwrap();
        std::os::unix::fs::symlink(&outside, project.join("leakdir")).unwrap();

        let hits = search_project(project.to_str().unwrap(), "AUDIT_SECRET_MARKER", 10);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.ends_with("visible.txt"));
        let _ = fs::remove_dir_all(base);
    }
}
