use crate::model::{KnowledgeGraph, KnowledgeKind, KnowledgeSchema};
use crate::parse::{edges_for, parse_document};
use std::path::{Path, PathBuf};

pub const MAX_FILES: usize = 512;
pub const MAX_FILE_BYTES: u64 = 512 * 1024;
pub const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_DEPTH: usize = 8;

/// Read-only discovery for one project. Files remain canonical; this function
/// never creates, rewrites or follows symlinks.
pub fn discover(
    root: &Path,
    schema: &KnowledgeSchema,
    project_id: &str,
    plugin_id: Option<&str>,
) -> Result<(KnowledgeGraph, Vec<String>), String> {
    let canonical = root
        .canonicalize()
        .map_err(|error| format!("Project folder is not available: {error}"))?;
    if !canonical.is_dir() {
        return Err("Project folder is not a directory".into());
    }
    let mut graph = KnowledgeGraph::default();
    let mut warnings = Vec::new();
    let mut budget = MAX_TOTAL_BYTES;
    let mut files = 0usize;
    for kind in &schema.kinds {
        let directory = canonical.join(&kind.directory);
        if !directory.is_dir() {
            continue;
        }
        walk(
            &canonical,
            &directory,
            kind,
            project_id,
            plugin_id,
            &mut graph,
            &mut warnings,
            &mut budget,
            &mut files,
            0,
        );
    }
    graph.objects.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.id.cmp(&b.id))
    });
    graph.objects.dedup_by(|a, b| a.id == b.id);
    graph.edges.sort_by(|a, b| {
        a.from_id
            .cmp(&b.from_id)
            .then_with(|| a.relation.cmp(&b.relation))
            .then_with(|| a.to_id.cmp(&b.to_id))
    });
    graph
        .edges
        .dedup_by(|a, b| a.from_id == b.from_id && a.to_id == b.to_id && a.relation == b.relation);
    Ok((graph, warnings))
}

#[allow(clippy::too_many_arguments)]
fn walk(
    base: &Path,
    directory: &Path,
    kind: &KnowledgeKind,
    project_id: &str,
    plugin_id: Option<&str>,
    graph: &mut KnowledgeGraph,
    warnings: &mut Vec<String>,
    budget: &mut usize,
    files: &mut usize,
    depth: usize,
) {
    if depth > MAX_DEPTH || *files >= MAX_FILES {
        return;
    }
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            warnings.push(format!(
                "{}: cannot read directory ({error})",
                directory.display()
            ));
            return;
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    for path in paths {
        if *files >= MAX_FILES {
            warnings.push(format!("Stopped after {MAX_FILES} knowledge files"));
            return;
        }
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.file_type().is_symlink() {
            warnings.push(format!("{}: symlink skipped", path.display()));
            continue;
        }
        if metadata.is_dir() {
            walk(
                base,
                &path,
                kind,
                project_id,
                plugin_id,
                graph,
                warnings,
                budget,
                files,
                depth + 1,
            );
            continue;
        }
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            continue;
        }
        if metadata.len() > MAX_FILE_BYTES {
            warnings.push(format!(
                "{}: larger than {} KiB and not indexed",
                path.display(),
                MAX_FILE_BYTES / 1024
            ));
            continue;
        }
        if metadata.len() as usize > *budget {
            warnings.push(format!(
                "{}: skipped because the discovery size budget was exhausted",
                path.display()
            ));
            continue;
        }
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                warnings.push(format!("{}: unreadable ({error})", path.display()));
                continue;
            }
        };
        if source.len() > MAX_FILE_BYTES as usize {
            warnings.push(format!("{}: grew beyond the size limit", path.display()));
            continue;
        }
        *budget = budget.saturating_sub(source.len());
        *files += 1;
        let relative = path
            .strip_prefix(base)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().to_string());
        let object = parse_document(kind, project_id, &relative, &source, plugin_id);
        let edges = edges_for(&object, &source);
        graph.objects.push(object);
        graph.edges.extend(edges);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FieldDef, FieldKind, RelationDef};
    use std::fs;

    fn schema() -> KnowledgeSchema {
        KnowledgeSchema {
            api: 1,
            kinds: vec![KnowledgeKind {
                id: "concept".into(),
                label: "Concept".into(),
                directory: "concepts".into(),
                statuses: vec!["proposed".into(), "accepted".into()],
                default_status: "proposed".into(),
                fields: vec![FieldDef {
                    id: "definition".into(),
                    label: "Definition".into(),
                    kind: FieldKind::Markdown,
                    required: false,
                    values: Vec::new(),
                }],
                relations: vec!["realizes".into()],
            }],
            relations: vec![RelationDef {
                id: "realizes".into(),
                label: "realizes".into(),
                from_kinds: vec!["concept".into()],
                to_kinds: vec!["concept".into()],
            }],
        }
    }

    #[test]
    fn discovery_reads_markdown_without_rewriting_and_skips_symlinks() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("concepts/nested")).unwrap();
        fs::write(
            root.path().join("concepts/workspace.md"),
            "---\nid: CON-001\ntitle: Workspace\nstatus: accepted\ndefinition: Shared place\nlinks:\n  - relation: realizes\n    target: CON-002\n---\n\nBody\n",
        )
        .unwrap();
        fs::write(
            root.path().join("concepts/nested/other.md"),
            "---\nid: CON-002\ntitle: Other\n---\n",
        )
        .unwrap();
        let before = fs::read_to_string(root.path().join("concepts/workspace.md")).unwrap();
        let (graph, warnings) = discover(root.path(), &schema(), "p", Some("governance")).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(graph.objects.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].to_id, "CON-002");
        assert_eq!(
            fs::read_to_string(root.path().join("concepts/workspace.md")).unwrap(),
            before
        );
    }
}
