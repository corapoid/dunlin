//! Bounded HEAD-only blame. Never attributes worktree/unsaved lines to authors;
//! clients map only unchanged lines from this immutable content to their buffer.
use super::{
    activity::{git_limited, git_object_limited},
    history::repository_root,
    operations::oid,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub oid: String,
    pub author: String,
    pub timestamp: i64,
    pub summary: String,
    pub original_line: usize,
    pub original_path: String,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Blame {
    pub head: String,
    pub path: String,
    pub contents: String,
    pub lines: Vec<Line>,
}
fn read(root: &str, args: &[&str], limit: u64) -> Result<String, String> {
    let mut options = vec!["--no-replace-objects", "-c", "core.fsmonitor=false"];
    options.extend_from_slice(args);
    git_limited(root, &options, limit)
}
pub fn load(path: &str, file: &str, expected_head: Option<&str>) -> Result<Blame, String> {
    let root = repository_root(path, None)?;
    // Git versions can open this multi-valued config eagerly, before a CLI
    // --ignore-revs-file "" reset. Refuse it instead of reading caller-selected
    // external files or caching attribution with an unpinned ignore-file state.
    let config = read(&root, &["config", "--null", "--list"], 1024 * 1024)?;
    if config.split_terminator('\0').any(|entry| {
        entry.split_once('\n').is_some_and(|(key, value)| {
            key.eq_ignore_ascii_case("blame.ignorerevsfile") && !value.is_empty()
        })
    }) {
        return Err("Blame with configured ignore-revs files is unsupported by the immutable HEAD cache; no ignore file was opened.".into());
    }
    if file.is_empty()
        || file.len() > 4096
        || !std::path::Path::new(file)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
        || file.contains('\0')
    {
        return Err("Blame paths must stay in the repository".into());
    }
    let head = read(&root, &["rev-parse", "--verify", "HEAD"], 1024)?
        .trim()
        .to_string();
    if !oid(&head) || expected_head.is_some_and(|h| h != head) {
        return Err("HEAD changed; refresh blame".into());
    }
    let tree = read(
        &root,
        &["--literal-pathspecs", "ls-tree", "-z", &head, "--", file],
        8192,
    )?;
    if !tree.starts_with("100644 ") && !tree.starts_with("100755 ") {
        return Err("Blame requires a tracked text file at HEAD".into());
    }
    let blob = tree
        .split_once('\t')
        .ok_or("Invalid tree record")?
        .0
        .split_whitespace()
        .nth(2)
        .ok_or("Missing blob")?;
    if !oid(blob) {
        return Err("Invalid blob identity".into());
    }
    let contents = read(&root, &["cat-file", "blob", blob], 2 * 1024 * 1024)?;
    if contents.contains('\0') {
        return Err("Blame is unavailable for binary files".into());
    }
    // Bare object-only mode avoids consulting an uncommitted/symlinked .mailmap
    // in the working tree. Ignore-revs files and replace refs cannot change the
    // meaning of a cache entry without changing its HEAD/file key.
    let git_dir = read(&root, &["rev-parse", "--absolute-git-dir"], 65536)?;
    let raw = git_object_limited(
        &root,
        &[
            "--no-replace-objects",
            "--git-dir",
            git_dir.trim_end_matches('\n'),
            "--bare",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "mailmap.file=/dev/null",
            "-c",
            "mailmap.blob=",
            "--literal-pathspecs",
            "blame",
            "--line-porcelain",
            "--no-progress",
            "--no-textconv",
            "--ignore-revs-file",
            "",
            &head,
            "--",
            file,
        ],
        16 * 1024 * 1024,
    )?;
    let lines = parse(&raw)?;
    let value = Blame {
        head,
        path: file.into(),
        contents,
        lines,
    };
    validate(&value)?;
    repository_root(path, Some(&root))?;
    Ok(value)
}
pub fn validate(value: &Blame) -> Result<(), String> {
    if !oid(&value.head)
        || value.path.is_empty()
        || value.contents.len() > 2 * 1024 * 1024
        || value.contents.contains('\0')
        || value.lines.len() > 20_000
        || value.contents.lines().count() != value.lines.len()
        || !value
            .contents
            .lines()
            .zip(&value.lines)
            .all(|(text, line)| text == line.text)
        || value.lines.iter().any(|l| {
            !oid(&l.oid)
                || l.original_line == 0
                || l.author.len() > 4096
                || l.summary.len() > 16384
                || l.original_path.len() > 8192
                || l.text.contains('\0')
        })
    {
        return Err("Invalid or oversized blame response".into());
    }
    Ok(())
}
fn parse(raw: &str) -> Result<Vec<Line>, String> {
    let mut input = raw.lines();
    let mut result = Vec::new();
    while let Some(header) = input.next() {
        let fields = header.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 3 || fields.len() > 4 || !oid(fields[0]) {
            return Err("Invalid blame header".into());
        }
        let mut line = Line {
            oid: fields[0].into(),
            original_line: fields[1].parse().map_err(|_| "Invalid blame line")?,
            author: String::new(),
            timestamp: 0,
            summary: String::new(),
            original_path: String::new(),
            text: String::new(),
        };
        let mut found = false;
        for value in input.by_ref() {
            if let Some(text) = value.strip_prefix('\t') {
                line.text = text.into();
                found = true;
                break;
            }
            if let Some(v) = value.strip_prefix("author ") {
                line.author = v.into();
            }
            if let Some(v) = value.strip_prefix("author-time ") {
                line.timestamp = v.parse().map_err(|_| "Invalid blame timestamp")?;
            }
            if let Some(v) = value.strip_prefix("summary ") {
                line.summary = v.into();
            }
            if let Some(v) = value.strip_prefix("filename ") {
                line.original_path = v.into();
            }
        }
        if !found || line.author.is_empty() {
            return Err("Incomplete blame record".into());
        }
        result.push(line);
        if result.len() > 20_000 {
            return Err("Blame exceeds 20,000 lines".into());
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    #[test]
    fn malformed_blame_is_never_silently_accepted() {
        assert!(super::parse("bad header\n\ttext").is_err());
        assert!(super::parse(&format!("{} 1 1\nauthor Test\n", "a".repeat(40))).is_err());
    }
}
