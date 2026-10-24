//! Lightweight project-wide text search.
//!
//! The UI hands the walk to GPUI's background executor, so large repositories
//! never block a frame. A search walks the project once, skips build/VCS
//! directories, only reads text files below 256 KiB and stops after `limit`
//! matches. The same implementation runs inside the relay, so remote search
//! behaves exactly like local search.

use std::fs;
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
    let mut hits = Vec::new();
    let mut scanned = 0usize;
    let mut bytes_scanned = 0u64;
    walk(
        Path::new(root),
        &needle,
        limit,
        &mut scanned,
        &mut bytes_scanned,
        0,
        &mut hits,
    );
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
        if path.is_dir() {
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
        if is_skipped_file(name) {
            continue;
        }
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        *bytes_scanned = bytes_scanned.saturating_add(meta.len());
        if *bytes_scanned > MAX_TOTAL_BYTES {
            return;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
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
        let root = std::env::temp_dir().join(format!("kestrel-search-{unique}"));
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
}
