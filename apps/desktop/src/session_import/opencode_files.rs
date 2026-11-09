//! Pre-SQLite OpenCode storage. Only session/message/part JSON is read.
use super::{Progress, adapters, collect_session_files};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};

pub(super) fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = std::env::var_os("OPENCODE_DATA_HOME") {
        roots.push(PathBuf::from(root));
    }
    if let Some(root) = std::env::var_os("XDG_DATA_HOME") {
        roots.push(PathBuf::from(root).join("opencode"));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.extend([
            home.join(".local/share/opencode"),
            home.join("Library/Application Support/opencode"),
            home.join(".opencode"),
        ]);
    }
    let mut seen = std::collections::HashSet::new();
    roots
        .into_iter()
        .filter_map(|root| root.canonicalize().ok())
        .filter(|root| seen.insert(root.clone()))
        .collect()
}

fn read(path: &Path) -> Result<Value, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(adapters::MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > adapters::MAX_FILE {
        return Err("OpenCode JSON record exceeds 16 MiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

pub(super) fn visit(
    storage: &Path,
    candidate: &mut dyn FnMut(adapters::Candidate) -> Result<(), String>,
    progress: &mut dyn FnMut(Progress),
) -> Result<Vec<String>, String> {
    let mut paths = Vec::new();
    let mut warnings = Vec::new();
    collect_session_files(
        &storage.join("session"),
        "json",
        &mut paths,
        &mut warnings,
        progress,
    );
    let total = paths.len();
    for (index, path) in paths.into_iter().enumerate() {
        let result = (|| {
            let info = read(&path)?;
            if info.get("parentID").is_some_and(|parent| !parent.is_null()) {
                return Ok(None);
            }
            let id = info["id"].as_str().ok_or("Missing OpenCode session ID")?;
            // IDs are filenames, never paths supplied by the source data.
            if id.contains(['/', '\\']) || id == "." || id == ".." {
                return Err("Invalid session ID".into());
            }
            let mut messages = Vec::new();
            let mut part_count = 0;
            let mut message_paths = Vec::new();
            if storage.join("message").join(id).is_dir() {
                collect_session_files(
                    &storage.join("message").join(id),
                    "json",
                    &mut message_paths,
                    &mut warnings,
                    &mut |_| {},
                );
            }
            message_paths.sort();
            if message_paths.len() > 30_000 {
                return Err("Too many OpenCode messages".into());
            }
            for path in message_paths {
                let meta = read(&path)?;
                let message_id = meta["id"].as_str().ok_or("Missing OpenCode message ID")?;
                if message_id.contains(['/', '\\']) || message_id == "." || message_id == ".." {
                    return Err("Invalid message ID".into());
                }
                let mut part_paths = Vec::new();
                let part_root = storage.join("part").join(message_id);
                if part_root.is_dir() {
                    collect_session_files(
                        &part_root,
                        "json",
                        &mut part_paths,
                        &mut warnings,
                        &mut |_| {},
                    );
                }
                part_paths.sort();
                part_count += part_paths.len();
                if part_count > 60_000 {
                    return Err("Too many OpenCode parts".into());
                }
                let parts = part_paths
                    .iter()
                    .map(|path| read(path))
                    .collect::<Result<Vec<_>, _>>()?;
                messages.push(serde_json::json!({"info":meta,"parts":parts}));
            }
            if messages.is_empty() {
                return Ok(None);
            }
            adapters::opencode(&serde_json::json!({"info":info,"messages":messages})).map(Some)
        })();
        match result {
            Ok(Some(row)) if !row.transcript.is_empty() => candidate(row)?,
            Ok(_) => {}
            Err(error) => warnings.push(format!(
                "{}: {error}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
        progress(Progress {
            phase: "Reading legacy OpenCode sessions",
            completed: index + 1,
            total: Some(total),
        });
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_pre_sqlite_v1_storage_without_changing_source_files() {
        let root = tempfile::tempdir().unwrap();
        for path in ["session/project", "message/ses_old", "part/msg_old"] {
            std::fs::create_dir_all(root.path().join(path)).unwrap();
        }
        let records = [
            (
                "session/project/ses_old.json",
                r#"{"id":"ses_old","directory":"/repo","title":"Legacy","time":{"updated":123}}"#,
            ),
            (
                "message/ses_old/msg_old.json",
                r#"{"id":"msg_old","role":"user","time":{"created":123}}"#,
            ),
            (
                "part/msg_old/part_old.json",
                r#"{"id":"part_old","type":"text","text":"Before SQLite"}"#,
            ),
        ];
        for (path, data) in records {
            std::fs::write(root.path().join(path), data).unwrap();
        }
        let mut sessions = Vec::new();
        let warnings = super::visit(
            root.path(),
            &mut |row| {
                sessions.push(row);
                Ok(())
            },
            &mut |_| {},
        )
        .unwrap();
        assert!(warnings.is_empty());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].source_id, "ses_old");
        assert!(
            matches!(&sessions[0].transcript[0], crate::harness::ChatItem::User { text, .. } if text == "Before SQLite")
        );
        for (path, data) in records {
            assert_eq!(
                std::fs::read_to_string(root.path().join(path)).unwrap(),
                data
            );
        }
    }
}
