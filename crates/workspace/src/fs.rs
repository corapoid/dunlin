//! Project-scoped filesystem access shared by the desktop and the relay.
//!
//! Every path is resolved against a project root and canonicalized before it is
//! used. That keeps `..`, absolute paths and symlinks that escape the project
//! out of both local and remote editing.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

/// The editor refuses to load anything larger than this.
pub const MAX_EDITOR_BYTES: u64 = 2 * 1024 * 1024;
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileContents {
    pub contents: String,
    pub readonly: bool,
    pub bytes: u64,
    pub mtime_ms: u64,
    pub binary: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileStat {
    pub bytes: u64,
    pub mtime_ms: u64,
    pub is_dir: bool,
}

/// Resolve `relative` inside `root`, rejecting traversal and escaping symlinks.
pub fn resolve(root: &str, relative: &str) -> Result<PathBuf, String> {
    let root_path = fs::canonicalize(root)
        .map_err(|error| format!("Project folder is not available: {error}"))?;
    if !root_path.is_dir() {
        return Err("Project folder is not a directory".into());
    }

    let relative = relative.trim();
    let relative = if relative.is_empty() { "." } else { relative };
    let candidate = Path::new(relative);
    if candidate.is_absolute() {
        return Err("Absolute paths are not allowed".into());
    }
    for component in candidate.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err("Path escapes the project folder".into()),
        }
    }

    let joined = root_path.join(candidate);
    let resolved = if joined.exists() {
        fs::canonicalize(&joined).map_err(|error| format!("Could not resolve path: {error}"))?
    } else {
        let parent = joined
            .parent()
            .ok_or_else(|| "Path has no parent folder".to_string())?;
        let parent = fs::canonicalize(parent)
            .map_err(|error| format!("Parent folder is not available: {error}"))?;
        if !parent.starts_with(&root_path) {
            return Err("Path escapes the project folder".into());
        }
        parent.join(
            joined
                .file_name()
                .ok_or_else(|| "Path has no file name".to_string())?,
        )
    };

    if !resolved.starts_with(&root_path) {
        return Err("Path escapes the project folder".into());
    }
    Ok(resolved)
}

/// Relative path from the project root, using `/` separators.
pub fn relative_to(root: &str, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn read_file(root: &str, relative: &str) -> Result<FileContents, String> {
    let path = resolve(root, relative)?;
    let metadata = fs::metadata(&path).map_err(|error| format!("Could not read file: {error}"))?;
    if metadata.is_dir() {
        return Err("This path is a folder".into());
    }
    if metadata.len() > MAX_EDITOR_BYTES {
        return Err(format!(
            "This file is too large to edit ({} KiB)",
            metadata.len() / 1024
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .map_err(|error| format!("Could not read file: {error}"))?
        .take(MAX_EDITOR_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read file: {error}"))?;
    if bytes.len() as u64 > MAX_EDITOR_BYTES {
        return Err("File grew beyond the editor size limit".into());
    }
    let binary = bytes.iter().take(BINARY_SNIFF_BYTES).any(|byte| *byte == 0);
    let mtime_ms = mtime_ms(&metadata);
    if binary {
        return Ok(FileContents {
            contents: "Binary file — editing is disabled.".into(),
            readonly: true,
            bytes: metadata.len(),
            mtime_ms,
            binary: true,
        });
    }
    match String::from_utf8(bytes) {
        Ok(contents) => Ok(FileContents {
            contents,
            readonly: false,
            bytes: metadata.len(),
            mtime_ms,
            binary: false,
        }),
        Err(_) => Ok(FileContents {
            contents: "File is not valid UTF-8 — editing is disabled.".into(),
            readonly: true,
            bytes: metadata.len(),
            mtime_ms,
            binary: true,
        }),
    }
}

pub fn write_file(root: &str, relative: &str, contents: &str) -> Result<FileStat, String> {
    if contents.len() as u64 > MAX_EDITOR_BYTES {
        return Err("This file is too large to save".into());
    }
    let path = resolve(root, relative)?;
    let metadata = fs::metadata(&path).map_err(|error| format!("Could not write file: {error}"))?;
    if metadata.is_dir() {
        return Err("This path is a folder".into());
    }
    fs::write(&path, contents).map_err(|error| format!("Could not save file: {error}"))?;
    stat_file(root, relative)
}

pub fn create_file(root: &str, relative: &str) -> Result<FileStat, String> {
    if Path::new(relative)
        .components()
        .any(|part| matches!(part, Component::Normal(name) if name.eq_ignore_ascii_case(".git")))
    {
        return Err("Files cannot be created inside Git metadata through Explorer".into());
    }
    let path = resolve(root, relative)?;
    let canonical_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    if path
        .strip_prefix(&canonical_root)
        .map_err(|error| error.to_string())?
        .components()
        .any(|part| matches!(part, Component::Normal(name) if name.eq_ignore_ascii_case(".git")))
    {
        return Err("Files cannot be created inside Git metadata through Explorer".into());
    }
    if path.exists() {
        return Err("A file with that name already exists".into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format!("Could not create file: {error}"))?;
    file.write_all(b"")
        .map_err(|error| format!("Could not create file: {error}"))?;
    stat_file(root, relative)
}

pub fn stat_file(root: &str, relative: &str) -> Result<FileStat, String> {
    let path = resolve(root, relative)?;
    let metadata = fs::metadata(&path).map_err(|error| format!("Could not stat file: {error}"))?;
    Ok(FileStat {
        bytes: metadata.len(),
        mtime_ms: mtime_ms(&metadata),
        is_dir: metadata.is_dir(),
    })
}

fn mtime_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{create_file, read_file, resolve, stat_file, write_file};
    use std::fs;

    fn temp_root(name: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dunlin-fs-{name}-{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn reads_and_writes_inside_the_project() {
        let root = temp_root("read-write");
        let root_text = root.to_string_lossy().to_string();
        create_file(&root_text, "notes.md").unwrap();
        write_file(&root_text, "notes.md", "hello\n").unwrap();
        let file = read_file(&root_text, "notes.md").unwrap();
        assert_eq!(file.contents, "hello\n");
        assert!(!file.readonly);
        assert_eq!(stat_file(&root_text, "notes.md").unwrap().bytes, 6);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        let root = temp_root("traversal");
        let root_text = root.to_string_lossy().to_string();
        assert!(resolve(&root_text, "../escape").is_err());
        assert!(resolve(&root_text, "/etc/passwd").is_err());
        assert!(resolve(&root_text, "src/../../escape").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn marks_binary_files_readonly() {
        let root = temp_root("binary");
        let root_text = root.to_string_lossy().to_string();
        fs::write(root.join("image.bin"), [0u8, 1, 2, 3]).unwrap();
        let file = read_file(&root_text, "image.bin").unwrap();
        assert!(file.binary);
        assert!(file.readonly);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn create_refuses_to_overwrite() {
        let root = temp_root("create");
        let root_text = root.to_string_lossy().to_string();
        create_file(&root_text, "new.txt").unwrap();
        assert!(create_file(&root_text, "new.txt").is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
